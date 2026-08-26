import { describe, expect, it } from "vitest";
import { formatLeagueTimestamp, leagueWinState, normalizeLeagueTimestamp } from "./leagueDisplay";

describe("league display helpers", () => {
  it("normalizes seconds, milliseconds, ISO strings and unknown timestamps", () => {
    expect(normalizeLeagueTimestamp(1700000000)).toBe(1700000000000);
    expect(normalizeLeagueTimestamp(1700000000000)).toBe(1700000000000);
    expect(normalizeLeagueTimestamp("2024-01-02T03:04:05Z")).toBe(Date.parse("2024-01-02T03:04:05Z"));
    expect(formatLeagueTimestamp(0)).toBe("比赛时间未知");
  });

  it("keeps unknown win values distinct from a loss", () => {
    expect(leagueWinState(true)).toBe(true);
    expect(leagueWinState(false)).toBe(false);
    expect(leagueWinState(undefined)).toBeNull();
  });
});
