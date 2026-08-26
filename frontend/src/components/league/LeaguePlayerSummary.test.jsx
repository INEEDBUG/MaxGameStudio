import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import LeaguePlayerSummary, { aggregatePlayerSummary } from "./LeaguePlayerSummary";

const now = Date.now();
const makeMatch = (win, teamId, offset = 0) => ({
  win, team_id: teamId, played_at: now - offset, kills: 8, deaths: 2, assists: 6,
  damage: 20000, damage_taken: 10000, gold: 12000, cs: 210, vision_score: 25,
  duration_seconds: 1800,
  participants: [
    { team_id: teamId, kills: 8, damage: 20000, damage_taken: 10000, gold: 12000, vision_score: 25 },
    { team_id: teamId, kills: 6, damage: 10000, damage_taken: 10000, gold: 8000, vision_score: 25 },
  ],
});

describe("LeaguePlayerSummary", () => {
  it("aggregates session, sides, streak, shares, and score", () => {
    const summary = aggregatePlayerSummary([makeMatch(true, 100), makeMatch(true, 200, 60_000)], now);
    expect(summary.winRate).toBe(1);
    expect(summary.streak).toEqual({ count: 2, winning: true });
    expect(summary.activeSession).toEqual({ games: 2, wins: 2 });
    expect(summary.blueSide).toBe(1);
    expect(summary.redSide).toBe(1);
    expect(summary.damageShare).toBeCloseTo(2 / 3);
    expect(summary.akariScore.maxScore).toBe(17);
  });

  it("renders the player summary surface", () => {
    render(<LeaguePlayerSummary matches={[makeMatch(true, 100), makeMatch(false, 200, 60_000)]}/>);
    expect(screen.getByText("玩家综合摘要")).toBeTruthy();
    expect(screen.getByText("Akari Score")).toBeTruthy();
    expect(screen.getByText("蓝 / 红方")).toBeTruthy();
  });
});
