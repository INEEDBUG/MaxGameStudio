import { describe, expect, it } from "vitest";
import { matchesLeagueRules, matchesLeagueRuleTree } from "./leagueMatchFilter";

const match = { champion_name:"Ashe", game_mode:"CLASSIC", position:"BOTTOM", kills:12, deaths:4, assists:8, damage:25000 };

describe("matchesLeagueRules", () => {
  it("combines numeric and text rules with AND or OR", () => {
    const rules=[{field:"kda",operator:"gte",value:"4"},{field:"champion_name",operator:"contains",value:"ash"}];
    expect(matchesLeagueRules(match,rules,"and")).toBe(true);
    expect(matchesLeagueRules(match,[...rules,{field:"damage",operator:"gte",value:"30000"}],"and")).toBe(false);
    expect(matchesLeagueRules(match,[{field:"kills",operator:"gte",value:"20"},{field:"position",operator:"eq",value:"bottom"}],"or")).toBe(true);
  });

  it("supports upstream-style game, item, spell and challenge predicates", () => {
    const rich={...match,win:true,game_type:"MATCHED_GAME",duration_seconds:240,items:[3006,6672],spell1_id:4,spell2_id:7,challenges:{killParticipation:0.72,soloKills:3}};
    expect(matchesLeagueRules(rich,[
      {field:"win",operator:"eq",value:"true"},
      {field:"is_remake",operator:"eq",value:"true"},
      {field:"has_item",operator:"eq",value:"3006"},
      {field:"has_spell",operator:"eq",value:"4"},
      {field:"kill_participation",operator:"gte",value:"0.7"},
      {field:"solo_kills",operator:"gte",value:"3"},
    ],"and")).toBe(true);
  });

  it("evaluates ally and enemy any/every participant scopes", () => {
    const scoped={...match,participant_puuid:"self",team_id:100,participants:[
      {puuid:"self",team_id:100,kills:12},
      {puuid:"ally",team_id:100,kills:3},
      {puuid:"enemy-1",team_id:200,kills:8},
      {puuid:"enemy-2",team_id:200,kills:5},
    ]};
    expect(matchesLeagueRules(scoped,[{scope:"any-enemies",field:"kills",operator:"gte",value:"8"}],"and")).toBe(true);
    expect(matchesLeagueRules(scoped,[{scope:"every-enemies",field:"kills",operator:"gte",value:"6"}],"and")).toBe(false);
    expect(matchesLeagueRules(scoped,[{scope:"every-allies",field:"kills",operator:"gte",value:"3"}],"and")).toBe(true);
  });

  it("supports arbitrarily nested AND, OR and NOT groups", () => {
    const tree={type:"group",logic:"and",children:[
      {type:"rule",field:"kills",operator:"gte",value:"10"},
      {type:"group",logic:"or",negate:true,children:[
        {type:"rule",field:"deaths",operator:"gte",value:"10"},
        {type:"rule",field:"champion_name",operator:"eq",value:"Garen"},
      ]},
    ]};
    expect(matchesLeagueRuleTree(match,tree)).toBe(true);
    expect(matchesLeagueRuleTree({...match,deaths:12},tree)).toBe(false);
  });
});
