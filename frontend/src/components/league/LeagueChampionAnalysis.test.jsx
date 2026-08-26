import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import LeagueChampionAnalysis, { aggregateChampionMatches, computeAkariScore } from "./LeagueChampionAnalysis";

const matches = [
  { champion_id: 1, champion_name: "安妮", win: true, kills: 10, deaths: 2, assists: 8, damage: 20000, damage_taken: 8000, gold: 12000, cs: 180, vision_score: 20, duration_seconds: 1800, team_id: 100, position: "MIDDLE", participants: [{ team_id: 100, damage: 20000, damage_taken: 8000, gold: 12000 }, { team_id: 100, damage: 10000, damage_taken: 12000, gold: 8000 }] },
  { champion_id: 1, champion_name: "安妮", win: false, kills: 4, deaths: 4, assists: 6, damage: 12000, damage_taken: 10000, gold: 9000, cs: 150, vision_score: 18, duration_seconds: 1800, team_id: 100, position: "MIDDLE", participants: [{ team_id: 100, damage: 12000, damage_taken: 10000, gold: 9000 }, { team_id: 100, damage: 12000, damage_taken: 10000, gold: 9000 }] },
  { champion_id: 2, champion_name: "奥拉夫", win: true, kills: 5, deaths: 5, assists: 5, damage: 9000, duration_seconds: 1800, team_id: 100, position: "JUNGLE", participants: [] },
];

describe("LeagueChampionAnalysis", () => {
  it("aggregates champion performance and team shares", () => {
    const [annie] = aggregateChampionMatches(matches);
    expect(annie.games).toBe(2);
    expect(annie.winRate).toBe(.5);
    expect(annie.kda).toBeCloseTo(14 / 3);
    expect(annie.positions.MIDDLE).toBe(2);
    expect(annie.damageShare).toBeCloseTo((2 / 3 + .5) / 2);
    expect(annie.akariScore.maxScore).toBe(17);
    expect(Number.isFinite(annie.akariScore.total)).toBe(true);
  });

  it("keeps the adapted Akari score capped at its upstream maximum", () => {
    const score = computeAkariScore(Array.from({ length: 8 }, () => ({ ...matches[0], win: true })));
    expect(score.total).toBeGreaterThan(0);
    expect(score.total).toBeLessThanOrEqual(score.maxScore);
    expect(typeof score.outstanding).toBe("boolean");
  });

  it("switches the visible champion analysis", () => {
    render(<LeagueChampionAnalysis matches={matches}/>);
    expect(screen.getAllByText("安妮").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: /奥拉夫/ }));
    expect(screen.getAllByText("奥拉夫").length).toBeGreaterThan(1);
    expect(screen.getByText("打野")).toBeTruthy();
  });
});
