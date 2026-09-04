import { beforeEach, describe, expect, it } from "vitest";
import {
  clearLeagueStartupPreference,
  LEAGUE_STARTUP_MODE_STORAGE_KEY,
  LEAGUE_STARTUP_REMEMBER_STORAGE_KEY,
  readLeagueStartupPreference,
  writeLeagueStartupPreference,
} from "./leagueStartupPreference";

describe("league startup preference", () => {
  beforeEach(() => localStorage.clear());

  it("persists all supported modes using the existing storage contract", () => {
    for (const mode of ["ask", "memory", "parallel"]) {
      expect(writeLeagueStartupPreference(mode)).toBe(true);
      expect(readLeagueStartupPreference()).toEqual({ mode, remembered: true, administrator: false });
    }
    expect(localStorage.getItem(LEAGUE_STARTUP_REMEMBER_STORAGE_KEY)).toBe("true");
  });

  it("rejects unknown values and restores ask by clearing the remembered mode", () => {
    expect(writeLeagueStartupPreference("unknown")).toBe(false);
    localStorage.setItem(LEAGUE_STARTUP_MODE_STORAGE_KEY, "memory");
    localStorage.setItem(LEAGUE_STARTUP_REMEMBER_STORAGE_KEY, "true");
    expect(clearLeagueStartupPreference()).toBe(true);
    expect(readLeagueStartupPreference()).toBeNull();
    expect(writeLeagueStartupPreference("ask")).toBe(true);
    expect(readLeagueStartupPreference()?.mode).toBe("ask");
  });

  it("removes persistence when the user chooses not to remember", () => {
    writeLeagueStartupPreference("parallel");
    expect(writeLeagueStartupPreference("memory", false)).toBe(true);
    expect(readLeagueStartupPreference()).toBeNull();
  });

  it("persists and clears the administrator launch choice with the remembered mode", () => {
    expect(writeLeagueStartupPreference("parallel", true, localStorage, { administrator: true })).toBe(true);
    expect(readLeagueStartupPreference()).toEqual({ mode: "parallel", remembered: true, administrator: true });
    expect(writeLeagueStartupPreference("memory", false, localStorage, { administrator: true })).toBe(true);
    expect(readLeagueStartupPreference()).toBeNull();
  });
});
