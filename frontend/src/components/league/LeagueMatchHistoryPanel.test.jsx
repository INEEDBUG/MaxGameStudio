import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import LeagueMatchHistoryPanel from "./LeagueMatchHistoryPanel";

vi.mock("./LeagueDetailedMatchCard", () => ({
  default: ({ match }) => <article data-testid="mock-history-card"><button type="button" aria-label="展开战绩详情">展开战绩详情</button><span>{match.game_id}</span></article>,
}));

const matches = [
  { game_id: 1003, played_at: 1_700_000_003_000, queue_id: 420, game_mode: "CLASSIC", win: true, champion_name: "安妮", participants: [] },
  { game_id: 1002, played_at: 1_700_000_002_000, queue_id: 450, game_mode: "ARAM", win: false, champion_name: "盖伦", participants: [] },
  { game_id: 1001, played_at: 1_700_000_001_000, queue_id: 420, game_mode: "CLASSIC", win: true, champion_name: "锤石", participants: [] },
];

describe("LeagueMatchHistoryPanel", () => {
  it("paginates locally and keeps the upstream-style card affordance", () => {
    render(<LeagueMatchHistoryPanel matches={matches} connected pageSize={2} />);

    expect(screen.getByText("1003")).toBeTruthy();
    expect(screen.getByText("1002")).toBeTruthy();
    expect(screen.getAllByTestId("mock-history-card")).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: "战绩下一页" }));
    expect(screen.getByText("1001")).toBeTruthy();
    expect(screen.getAllByTestId("mock-history-card")).toHaveLength(1);
  });

  it("filters by queue and result, then clears the filter", () => {
    render(<LeagueMatchHistoryPanel matches={matches} connected pageSize={20} />);

    fireEvent.change(screen.getByRole("combobox", { name: "战绩队列筛选" }), { target: { value: "420" } });
    expect(screen.getByText("1003")).toBeTruthy();
    expect(screen.getByText("1001")).toBeTruthy();
    expect(screen.queryByText("1002")).toBeNull();
    fireEvent.change(screen.getByRole("combobox", { name: "战绩结果筛选" }), { target: { value: "loss" } });
    expect(screen.getByTestId("league-history-empty")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "清空筛选" }).at(-1));
    expect(screen.getByText("1002")).toBeTruthy();
  });

  it("shows a non-empty loading state and a useful empty state", () => {
    const { rerender } = render(<LeagueMatchHistoryPanel matches={[]} busy connected />);
    expect(screen.getByTestId("league-history-loading")).toBeTruthy();
    rerender(<LeagueMatchHistoryPanel matches={[]} busy={false} connected />);
    expect(screen.getByTestId("league-history-empty")).toBeTruthy();
    expect(screen.getByText("暂无可用战绩")).toBeTruthy();
  });

  it("renders the LeagueAkari-style profile sidebar without changing the match list contract", () => {
    const onOpenPlayer = vi.fn();
    render(<LeagueMatchHistoryPanel matches={matches} currentPlayer={{ puuid: "self", game_name: "Tester", tag_line: "HN1", profile_icon_id: 1 }} connected onOpenPlayer={onOpenPlayer} />);

    expect(screen.getByTestId("league-history-sidebar")).toBeTruthy();
    expect(screen.getByText("Tester")).toBeTruthy();
    expect(screen.getByText("近期总览")).toBeTruthy();
    expect(screen.getByText("英雄熟练度")).toBeTruthy();
    expect(screen.getByText("近期共同对局")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "打开完整玩家中心" }));
    expect(onOpenPlayer).toHaveBeenCalledWith("self");
  });

  it("keeps a history avatar when the refreshed current summoner temporarily omits it", () => {
    const history = [{
      ...matches[0],
      participants: [{
        puuid: "self",
        game_name: "Tester",
        profile_icon_id: 29,
        champion_id: 22,
        team_id: 100,
        win: true,
      }],
    }];
    render(<LeagueMatchHistoryPanel matches={history} currentPlayer={{ puuid: "self", game_name: "Tester", profile_icon_id: null }} connected />);

    expect(screen.getByAltText("玩家头像").getAttribute("src")).toContain("/api/league-lab/assets/profile-icons/29.jpg");
  });

  it("uses the loaded page size when the server omits an explicit total", () => {
    render(<LeagueMatchHistoryPanel matches={matches} connected pageInfo={{ page: 1, page_size: 20, has_more: true }} />);

    expect(screen.getByText("共 3 场")).toBeTruthy();
    expect(screen.queryByText("共 0 场")).toBeNull();
  });
});
