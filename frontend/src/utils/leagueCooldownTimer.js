export function createLeagueSpellTimer({ timerType, cooldownSeconds, abilityHaste = 0, now = Date.now() }) {
  if (timerType === "countdown" && Number(cooldownSeconds) > 0) {
    const haste = Math.max(0, Number(abilityHaste) || 0);
    const durationMs = Number(cooldownSeconds) * (100 / (100 + haste)) * 1000;
    return { type: "countdown", baseAt: now + durationMs };
  }
  return { type: "countup", baseAt: now };
}

export function adjustLeagueSpellTimer(timer, deltaY, reverseAdjustment = false, now = Date.now()) {
  if (!timer) return null;
  const timeDelta = (reverseAdjustment ? -1 : 1) * Number(deltaY || 0) * 50;
  const baseAt = timer.type === "countup"
    ? Math.min(timer.baseAt + timeDelta, now)
    : Math.max(timer.baseAt + timeDelta, now);
  return { ...timer, baseAt };
}

export function formatLeagueSpellTimer(timer, now = Date.now()) {
  if (!timer || timer.baseAt == null) return "";
  const seconds = (timer.type === "countdown" ? timer.baseAt - now : now - timer.baseAt) / 1000;
  if (seconds < 0) return "OK";
  if (seconds > 999) return "999";
  return seconds < 10 ? seconds.toFixed(1) : String(Math.floor(seconds));
}

export function buildLeagueTimerChatText({ playerName, spellName, timer, gameTimeSeconds, now = Date.now() }) {
  if (!timer || gameTimeSeconds == null || !playerName || !spellName) return "";
  const eventAtMs = Math.max(0, timer.baseAt - now + Number(gameTimeSeconds) * 1000);
  const totalSeconds = Math.floor(eventAtMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  const clock = `${minutes}:${String(seconds).padStart(2, "0")}`;
  return timer.type === "countdown"
    ? `${playerName} ${spellName} ${clock} 就绪`
    : `${playerName} ${spellName} ${clock} 已使用`;
}
