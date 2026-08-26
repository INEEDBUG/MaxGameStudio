export function normalizeLeagueTimestamp(value) {
  if (value == null || value === "") return null;
  if (typeof value === "string" && !/^-?\d+(\.\d+)?$/.test(value.trim())) {
    const parsed = Date.parse(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) return null;
  return numeric < 1e12 ? numeric * 1000 : numeric;
}

export function formatLeagueTimestamp(value) {
  const timestamp = normalizeLeagueTimestamp(value);
  if (timestamp == null) return "比赛时间未知";
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? "比赛时间未知" : date.toLocaleString("zh-CN", { hour12: false });
}

export function leagueWinState(value) {
  if (value === true || value === 1 || value === "1" || value === "true") return true;
  if (value === false || value === 0 || value === "0" || value === "false") return false;
  return null;
}
