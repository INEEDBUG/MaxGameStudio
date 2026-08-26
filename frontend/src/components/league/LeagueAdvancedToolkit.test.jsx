import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueAdvancedToolkit from "./LeagueAdvancedToolkit";
import { createLeagueQueueLobby, fetchLeagueGamePreview, fetchLeagueMatchDetails, runLeagueProfileUtilityAction } from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  fetchLeagueLobbyOptions: vi.fn().mockResolvedValue({
    queues: [{ id: 420, name: "单双排", eligible: true }],
    lobby: { gameConfig: { queueId: 420 } },
    strawberry: { active: false, maps: [], difficulties: [1,2,3], loadout_available: false },
  }),
  fetchLeagueChampions: vi.fn().mockResolvedValue({ champions: [{ id: 22, name: "艾希" }] }),
  fetchLeagueProfileSkins: vi.fn().mockResolvedValue({ skins: [] }),
  fetchLeagueGamePreview: vi.fn().mockResolvedValue({
    source: "lcu",
    metadata: { game_id: 123, game_mode: "CLASSIC" },
    timeline: { loaded: true, frame_count: 2, event_count: 1 },
    teams: [],
    ongoing_preview: { historical_preview: true, game_id: 123, available: true, players: [] },
  }),
  fetchLeagueMatchDetails: vi.fn().mockResolvedValue({
    source: "lcu",
    game_id: 123,
    map_id: 11,
    frame_count: 2,
    event_count: 1,
    participants: [],
    frames: [],
    events: [],
  }),
  createLeagueQueueLobby: vi.fn().mockResolvedValue({ created: true }),
  leaveLeagueLobby: vi.fn(),
  updateLeagueStrawberryPlayer: vi.fn(),
  updateLeagueStrawberryMap: vi.fn(),
  updateLeagueStrawberryDifficulty: vi.fn(),
  updateLeagueProfileBackground: vi.fn(),
  runLeagueProfileUtilityAction: vi.fn().mockResolvedValue({ applied: true }),
}));

const props={busy:false,onBusyChange:vi.fn(),onError:vi.fn()};

describe("LeagueAdvancedToolkit",()=>{
  beforeEach(()=>{vi.clearAllMocks();window.prompt=vi.fn();});

  it("keeps lobby and profile writes disabled until the account gate is enabled",async()=>{
    render(<LeagueAdvancedToolkit {...props} enabled={false}/>);
    await screen.findByText(/单双排/);
    fireEvent.change(screen.getByRole("combobox",{name:"房间队列"}),{target:{value:"420"}});
    expect(screen.getByRole("button",{name:"创建房间"}).disabled).toBe(true);
    expect(screen.getByRole("button",{name:"清空全部表情槽位"}).disabled).toBe(true);
  });

  it("creates only the selected eligible queue after the exact phrase",async()=>{
    render(<LeagueAdvancedToolkit {...props} enabled/>);
    await screen.findByText(/单双排/);
    fireEvent.change(screen.getByRole("combobox",{name:"房间队列"}),{target:{value:"420"}});
    window.prompt.mockReturnValueOnce("我确认创建");
    fireEvent.click(screen.getByRole("button",{name:"创建房间"}));
    await waitFor(()=>expect(createLeagueQueueLobby).toHaveBeenCalledWith(420,"我确认创建"));
  });

  it("requires the profile modification phrase for utility actions",async()=>{
    render(<LeagueAdvancedToolkit {...props} enabled/>);
    await screen.findByText(/单双排/);
    window.prompt.mockReturnValueOnce("错误");
    fireEvent.click(screen.getByRole("button",{name:"清空全部表情槽位"}));
    expect(runLeagueProfileUtilityAction).not.toHaveBeenCalled();
    window.prompt.mockReturnValueOnce("我确认修改");
    fireEvent.click(screen.getByRole("button",{name:"清空全部表情槽位"}));
    await waitFor(()=>expect(runLeagueProfileUtilityAction).toHaveBeenCalledWith("clear-emotes","我确认修改"));
  });

  it("loads an arbitrary game and passes the exact read-only draft to the ongoing panel without preloading details",async()=>{
    const onDryRunGame=vi.fn();
    render(<LeagueAdvancedToolkit {...props} enabled={false} onDryRunGame={onDryRunGame}/>);
    await screen.findByText(/单双排/);
    fireEvent.change(screen.getByRole("textbox",{name:"Game ID"}),{target:{value:"123"}});
    fireEvent.click(screen.getByRole("button",{name:"查看对局"}));
    await waitFor(()=>expect(fetchLeagueGamePreview).toHaveBeenCalledWith(123,"auto",true));
    expect(fetchLeagueMatchDetails).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole("button",{name:"载入实时面板模拟"}));
    expect(onDryRunGame).toHaveBeenCalledWith({historical_preview:true,game_id:123,available:true,players:[]});
  });

  it("loads full details on demand, renders the detailed card and routes player clicks",async()=>{
    const onOpenPlayer=vi.fn();
    fetchLeagueGamePreview.mockResolvedValueOnce({
      source: "lcu",
      metadata: { game_id: 123, game_mode: "CLASSIC", duration_seconds: 1800 },
      timeline: { loaded: true, frame_count: 2, event_count: 1 },
      teams: [{ team_id: 100, win: true, players: [
        { participant_id: 1, puuid: "self", team: 100, champion_id: 1, champion_name: "安妮", summoner: { gameName: "自己" }, match_stats: { kills: 8, deaths: 2, assists: 7, damage: 15000, cs: 140, gold: 11000, win: true } },
        { participant_id: 2, puuid: "ally", team: 100, champion_id: 2, champion_name: "奥拉夫", summoner: { gameName: "队友" }, match_stats: { kills: 5, deaths: 4, assists: 6, damage: 12000, cs: 130, gold: 9000, win: true } },
      ] }, { team_id: 200, win: false, players: [
        { participant_id: 6, puuid: "enemy", team: 200, champion_id: 3, champion_name: "加里奥", summoner: { gameName: "对手" }, match_stats: { kills: 4, deaths: 8, assists: 3, damage: 9000, cs: 150, gold: 8000, win: false } },
      ] }],
      ongoing_preview: { historical_preview: true, game_id: 123, available: true, players: [] },
    });
    fetchLeagueMatchDetails.mockResolvedValueOnce({ source: "lcu", game_id: 123, map_id: 11, participants: [
      { participant_id: 1, puuid: "self", team_id: 100, champion_id: 1, game_name: "自己" },
      { participant_id: 2, puuid: "ally", team_id: 100, champion_id: 2, game_name: "队友" },
      { participant_id: 6, puuid: "enemy", team_id: 200, champion_id: 3, game_name: "对手" },
    ], frames: [], events: [], frame_count: 0, event_count: 0 });
    render(<LeagueAdvancedToolkit {...props} enabled={false} onOpenPlayer={onOpenPlayer}/>);
    await screen.findByText(/单双排/);
    fireEvent.change(screen.getByRole("textbox",{name:"Game ID"}),{target:{value:"123"}});
    fireEvent.click(screen.getByRole("button",{name:"查看对局"}));
    await screen.findByRole("button",{name:"查看完整详情"});
    fireEvent.click(screen.getByRole("button",{name:"查看完整详情"}));
    await waitFor(()=>expect(fetchLeagueMatchDetails).toHaveBeenCalledWith(123,"auto"));
    expect(await screen.findByText("完整对局详情")).toBeTruthy();
    fireEvent.click(screen.getByRole("button",{name:"展开战绩详情"}));
    expect(await screen.findByText("队伍 100 · 胜利")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button",{name:/队友/}).at(-1));
    expect(onOpenPlayer).toHaveBeenCalledWith("ally");
    fireEvent.click(screen.getByRole("button",{name:"时间线"}));
    expect(fetchLeagueMatchDetails).toHaveBeenCalledTimes(1);
  });

  it("keeps an inline error when full details cannot be loaded",async()=>{
    fetchLeagueMatchDetails.mockRejectedValueOnce({ response: { data: { detail: "详情服务暂不可用" } } });
    render(<LeagueAdvancedToolkit {...props} enabled={false}/>);
    await screen.findByText(/单双排/);
    fireEvent.change(screen.getByRole("textbox",{name:"Game ID"}),{target:{value:"123"}});
    fireEvent.click(screen.getByRole("button",{name:"查看对局"}));
    fireEvent.click(await screen.findByRole("button",{name:"查看完整详情"}));
    expect((await screen.findByRole("alert")).textContent).toContain("详情服务暂不可用");
  });
});
