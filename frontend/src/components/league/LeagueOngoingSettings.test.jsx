import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import LeagueOngoingSettings from "./LeagueOngoingSettings";

const settings = {
  ongoing_query_in_lobby_phase: true,
  ongoing_show_jungle_pathing_for_all_players: false,
  ongoing_show_match_history_item_border: false,
  ongoing_order_player_by: "default",
  ongoing_champion_usage_mode: "recent",
  ongoing_show_champion_usage: true,
  ongoing_match_history_tag_preference: "current",
  ongoing_game_details_load_count: 20,
  ongoing_player_card_tag_settings: { "average-team-damage": false },
};

describe("LeagueOngoingSettings", () => {
  it("persists the upstream-equivalent lobby and player-card options", () => {
    const onUpdate = vi.fn();
    render(<LeagueOngoingSettings settings={settings} onUpdate={onUpdate}/>);

    fireEvent.click(screen.getByRole("switch", { name: "在房间阶段分析队友" }));
    fireEvent.change(screen.getByLabelText("实时玩家排序"), { target: { value: "akari-score" } });
    fireEvent.change(screen.getByLabelText("实时英雄数据来源"), { target: { value: "mastery" } });
    fireEvent.change(screen.getByLabelText("实时战绩样本范围"), { target: { value: "all" } });
    fireEvent.change(screen.getByLabelText("实时详情时间线数量"), { target: { value: "8" } });
    fireEvent.click(screen.getByLabelText("显示团队输出占比标签"));

    expect(onUpdate).toHaveBeenCalledWith({ ongoing_query_in_lobby_phase: false });
    expect(onUpdate).toHaveBeenCalledWith({ ongoing_order_player_by: "akari-score" });
    expect(onUpdate).toHaveBeenCalledWith({ ongoing_champion_usage_mode: "mastery", ongoing_show_champion_usage: true });
    expect(onUpdate).toHaveBeenCalledWith({ ongoing_match_history_tag_preference: "all" });
    expect(onUpdate).toHaveBeenCalledWith({ ongoing_game_details_load_count: 8 });
    expect(onUpdate).toHaveBeenCalledWith({ ongoing_player_card_tag_settings: { "average-team-damage": true } });
  });
});
