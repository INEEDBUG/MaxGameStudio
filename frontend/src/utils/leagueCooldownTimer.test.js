import { describe, expect, it } from "vitest";
import {
  adjustLeagueSpellTimer,
  buildLeagueTimerChatText,
  createLeagueSpellTimer,
  formatLeagueSpellTimer,
} from "./leagueCooldownTimer";

describe("league cooldown timer", () => {
  it("applies summoner spell haste to countdown duration", () => {
    expect(createLeagueSpellTimer({ timerType: "countdown", cooldownSeconds: 300, abilityHaste: 100, now: 1000 }))
      .toEqual({ type: "countdown", baseAt: 151000 });
  });

  it("starts countup timers at the current instant", () => {
    expect(createLeagueSpellTimer({ timerType: "countup", cooldownSeconds: 300, now: 5000 }))
      .toEqual({ type: "countup", baseAt: 5000 });
  });

  it("adjusts in both wheel directions without crossing the present", () => {
    const timer = { type: "countdown", baseAt: 20000 };
    expect(adjustLeagueSpellTimer(timer, -20, false, 10000).baseAt).toBe(19000);
    expect(adjustLeagueSpellTimer(timer, -20, true, 10000).baseAt).toBe(21000);
    expect(adjustLeagueSpellTimer(timer, -1000, false, 10000).baseAt).toBe(10000);
  });

  it("formats active, complete and countup timers", () => {
    expect(formatLeagueSpellTimer({ type: "countdown", baseAt: 19500 }, 10000)).toBe("9.5");
    expect(formatLeagueSpellTimer({ type: "countdown", baseAt: 9000 }, 10000)).toBe("OK");
    expect(formatLeagueSpellTimer({ type: "countup", baseAt: 0 }, 12000)).toBe("12");
  });

  it("builds game-clock messages and clamps pre-game time to zero", () => {
    expect(buildLeagueTimerChatText({
      playerName: "阿狸",
      spellName: "闪现",
      timer: { type: "countdown", baseAt: 70000 },
      gameTimeSeconds: 30,
      now: 10000,
    })).toBe("阿狸 闪现 1:30 就绪");
    expect(buildLeagueTimerChatText({
      playerName: "阿狸",
      spellName: "传送",
      timer: { type: "countup", baseAt: 0 },
      gameTimeSeconds: 0,
      now: 10000,
    })).toBe("阿狸 传送 0:00 已使用");
  });
});
