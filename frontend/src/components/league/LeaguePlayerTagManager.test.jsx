import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeaguePlayerTagManager, { buildLeaguePlayerTagsExport, parseLeaguePlayerTagsImport } from "./LeaguePlayerTagManager";

const mocks = vi.hoisted(() => ({
  fetch: vi.fn(),
  update: vi.fn(),
  remove: vi.fn(),
  importRows: vi.fn(),
}));

vi.mock("../../api/leagueLabApi", () => ({
  fetchLeaguePlayerTags: mocks.fetch,
  updateLeaguePlayerTag: mocks.update,
  deleteLeaguePlayerTag: mocks.remove,
  importLeaguePlayerTags: mocks.importRows,
}));
vi.mock("../../api/api", () => ({ getLeagueProfileIconUrl: (id) => `/profile/${id}` }));

const row = {
  key: "owner::target",
  owner_puuid: "owner",
  puuid: "target",
  player: { game_name: "Player", tag_line: "CN1", profile_icon_id: 12 },
  tag: { label: "可靠队友", note: "会沟通", color: "emerald" },
};

describe("LeaguePlayerTagManager", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.fetch.mockReset().mockResolvedValue({ rows: [row], total: 1, page: 1 });
    mocks.update.mockReset().mockResolvedValue({});
    mocks.remove.mockReset().mockResolvedValue({});
    mocks.importRows.mockReset().mockResolvedValue({ imported: 1 });
  });

  it("uses a versioned import format and bounds imported values", () => {
    const exported = buildLeaguePlayerTagsExport([row], "2026-08-15T00:00:00.000Z");
    expect(exported.format).toBe("max-game-studio/league-player-tags");
    expect(exported.rows[0]).toMatchObject({ owner_puuid: "owner", puuid: "target", label: "可靠队友" });
    expect(parseLeaguePlayerTagsImport(JSON.stringify(exported))).toHaveLength(1);
    expect(() => parseLeaguePlayerTagsImport("{}" )).toThrow("无法识别");
  });

  it("lists and edits an account-scoped tag", async () => {
    render(<LeaguePlayerTagManager/>);
    expect(await screen.findByText("Player#CN1")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("编辑 Player#CN1"));
    fireEvent.change(screen.getByLabelText("标签名称"), { target: { value: "宿敌" } });
    fireEvent.click(screen.getByText("保存"));
    await waitFor(() => expect(mocks.update).toHaveBeenCalledWith("owner::target", expect.objectContaining({ label: "宿敌" })));
  });

  it("masks the manager in streamer mode until explicitly revealed", async () => {
    render(<LeaguePlayerTagManager streamerMode/>);
    expect(screen.getByText("直播隐私模式已遮挡玩家标签管理")).toBeTruthy();
    expect(mocks.fetch).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("仅本次查看"));
    expect(await screen.findByText("Player#CN1")).toBeTruthy();
  });
});
