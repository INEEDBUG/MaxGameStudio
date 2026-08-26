const ALIASES = [
  "星界旅人", "峡谷旅人", "符文守望", "云顶来客", "河道信使",
  "蓝方先锋", "红方先锋", "迷雾行者", "水晶守卫", "远古回声",
];

function stableIndex(seed, size) {
  let hash = 2166136261;
  for (const char of String(seed || "")) {
    hash ^= char.codePointAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash) % size;
}

export function maskLeagueName(value, index = 0, useAlias = false, seed = "") {
  if (!useAlias) return `召唤师 ${index + 1}`;
  const basis = seed || value || String(index);
  return `${ALIASES[stableIndex(basis, ALIASES.length)]} ${index + 1}`;
}

export function leaguePrivacyText(value, enabled, replacement = "●●●●●●") {
  return enabled && value ? replacement : value;
}
