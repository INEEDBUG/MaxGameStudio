import { describe, expect, it } from "vitest";
import {
  buildLeagueFormPreset,
  buildLeagueJunglePreset,
  buildLeaguePremadePreset,
  buildRatingLines,
  getLeaguePresetOptions,
  normalizeLeaguePresetOptions,
  selectLeaguePresetPlayers,
  serializeLeaguePresetOptions,
} from "./leagueChatPresets";

const players = [
  { summoner:{gameName:"Alpha"}, champion_name:"阿狸", recent:{matches:10,wins:6}, champion_usage:{matches:4,average_kda:3.125}, premade_group:1 },
  { summoner:{gameName:"Beta"}, champion_name:"盖伦", recent:{matches:5,wins:2}, champion_usage:{matches:2,average_kda:1.5}, premade_group:1 },
];

describe("League chat preset generators", () => {
  it("builds recent-form lines without identifiers", () => {
    expect(buildLeagueFormPreset(players)[0]).toBe("Alpha：近10场 60%胜率，阿狸 4场 / KDA 3.13");
  });

  it("groups inferred premades", () => {
    expect(buildLeaguePremadePreset(players)).toEqual(["组排 A：Alpha、Beta"]);
  });

  it("only includes players with an analyzed jungle draft", () => {
    const enriched=[...players,{summoner:{gameName:"Jungle"},jungle_analysis:{games_analyzed:4,draft:"红开后偏向下路"}}];
    expect(buildLeagueJunglePreset(enriched)).toEqual(["Jungle：红开后偏向下路"]);
  });

  it("normalizes backend snake_case options without losing the LeagueAkari defaults", () => {
    const options = getLeaguePresetOptions({
      in_game_rating_preset_options: {
        target_mode: "enemy",
        selected_puuids: ["enemy"],
        name_display_strategy: "preferName",
        show_current_champion: true,
        display: { win_rate: false, avg_vision_score: true },
      },
    }, "rating");
    expect(options.targetMode).toBe("enemy");
    expect(options.nameDisplayStrategy).toBe("preferName");
    expect(options.showCurrentChampion).toBe(true);
    expect(options.display.winRate).toBe(false);
    expect(options.display.avgVisionScore).toBe(true);
    expect(options.display.kda).toBe(true);
    expect(serializeLeaguePresetOptions("rating", options)).toEqual(expect.objectContaining({
      target_mode: "enemy",
      selected_puuids: ["enemy"],
      name_display_strategy: "preferName",
      show_current_champion: true,
    }));
  });

  it("uses one generator for target selection and the saved metric switches", () => {
    const options = normalizeLeaguePresetOptions("rating", {
      targetMode: "friendly",
      display: { winRate: true, kda: false, mainChampions: false, mainPositions: false },
    });
    const selected = selectLeaguePresetPlayers([
      { puuid: "self", team: 100, summoner: { gameName: "Self" }, rating_summary: { win_rate: 1 } },
      { puuid: "enemy", team: 200, summoner: { gameName: "Enemy" }, rating_summary: { win_rate: 0 } },
    ], options, "self");
    expect(buildRatingLines(selected, options)).toEqual(["Self：胜率 100%"]);
  });
});
