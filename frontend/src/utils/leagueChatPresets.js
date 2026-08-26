import { maskLeagueName } from "./leagueStreamerMode";

// These option names intentionally mirror LeagueAkari's in-game-send pane.
// The backend stores the same values in snake_case; these helpers accept both
// shapes so every caller uses one deterministic generator.
export const TARGET_OPTIONS = [
  ["all", "双方全部"],
  ["friendly", "仅己方"],
  ["enemy", "仅敌方"],
  ["selected", "手动选择"],
];

export const SHORTCUT_TARGETS = [["friendly", "己方"], ["enemy", "敌方"], ["all", "双方"]];

export const NAME_STRATEGIES = [
  ["preferName", "优先玩家名"],
  ["preferChampionName", "优先当前英雄"],
  ["championNameWithName", "英雄 · 玩家名"],
];

export const RATING_OPTIONS = [
  ["winRate", "胜率", "近期样本胜率"],
  ["kda", "平均 KDA", "近期样本平均 KDA"],
  ["avgSoloKills", "场均单杀", "每场样本都提供单杀字段时显示"],
  ["avgVisionScore", "场均视野", "近期样本平均视野得分"],
  ["avgChampionDamage", "团队输出占比", "英雄伤害占团队总伤害比例"],
  ["avgDamageTaken", "团队承伤占比", "承伤占团队总承伤比例"],
  ["avgGold", "团队经济占比", "经济占团队总经济比例"],
  ["avgCsPerMinute", "每分钟补刀", "仅使用有效时长样本"],
  ["avgKillParticipation", "参团率", "击杀与助攻占团队击杀比例"],
  ["avgDamageGoldEfficiency", "伤害经济效率", "英雄伤害除以经济"],
  ["mainChampions", "主力英雄", "近期样本使用最多的英雄"],
  ["mainPositions", "主位置", "近期样本出现最多的位置"],
];

export const JUNGLE_OPTIONS = [
  ["activityPreference", "活动区域", "上、中、下半区活动偏好"],
  ["firstClearDistribution", "首开分布", "首个野区营地偏好"],
  ["earlyGank", "早期参与", "3/4 分钟参与击杀率"],
  ["dragonControl", "龙控制", "首条龙率、首龙时间和场均小龙"],
  ["monsterControl", "史诗野怪控制", "场均虚空巢虫、先锋和男爵"],
  ["mainChampions", "主力打野英雄", "近期打野样本使用最多的英雄"],
];

const RATING_DISPLAY_KEYS = {
  winRate: "win_rate",
  kda: "kda",
  avgSoloKills: "avg_solo_kills",
  avgVisionScore: "avg_vision_score",
  avgChampionDamage: "avg_champion_damage",
  avgDamageTaken: "avg_damage_taken",
  avgGold: "avg_gold",
  avgCsPerMinute: "avg_cs_per_minute",
  avgKillParticipation: "avg_kill_participation",
  avgDamageGoldEfficiency: "avg_damage_gold_efficiency",
  mainChampions: "main_champions",
  mainPositions: "main_positions",
};

const JUNGLE_DISPLAY_KEYS = {
  activityPreference: "activity_preference",
  firstClearDistribution: "first_clear_distribution",
  earlyGank: "early_gank",
  dragonControl: "dragon_control",
  monsterControl: "monster_control",
  mainChampions: "main_champions",
};

const VALID_TARGET_MODES = new Set(TARGET_OPTIONS.map(([value]) => value));
const VALID_NAME_STRATEGIES = new Set(NAME_STRATEGIES.map(([value]) => value));

export function createDefaultLeaguePresetOptions() {
  return {
    rating: {
      targetMode: "all",
      selectedPuuids: [],
      nameDisplayStrategy: "preferChampionName",
      showCurrentChampion: false,
      display: Object.fromEntries(RATING_OPTIONS.map(([key]) => [key, ["winRate", "kda", "avgSoloKills", "mainChampions", "mainPositions"].includes(key)])),
    },
    jungle: {
      targetMode: "all",
      selectedPuuids: [],
      nameDisplayStrategy: "preferChampionName",
      showCurrentChampion: true,
      display: Object.fromEntries(JUNGLE_OPTIONS.map(([key]) => [key, true])),
    },
    premade: {
      targetMode: "all",
      selectedPuuids: [],
      nameDisplayStrategy: "preferName",
    },
  };
}

export function presetOptionsKey(kind) {
  return `in_game_${kind}_preset_options`;
}

export function shortcutSettingsKey(kind) {
  return `in_game_${kind}_shortcuts`;
}

function readOption(raw, camelKey, snakeKey = camelKey) {
  if (!raw || typeof raw !== "object") return undefined;
  return raw[camelKey] !== undefined ? raw[camelKey] : raw[snakeKey];
}

function normalizeDisplay(defaults, raw, aliases) {
  const source = raw && typeof raw === "object" ? raw : {};
  return Object.fromEntries(Object.entries(defaults).map(([camelKey, defaultValue]) => {
    const snakeKey = aliases[camelKey] || camelKey;
    const value = source[camelKey] !== undefined ? source[camelKey] : source[snakeKey];
    return [camelKey, typeof value === "boolean" ? value : defaultValue];
  }));
}

/** Converts backend model_dump() data or the legacy renderer draft. */
export function normalizeLeaguePresetOptions(kind, raw) {
  const allDefaults = createDefaultLeaguePresetOptions();
  const defaults = allDefaults[kind] || allDefaults.rating;
  const source = raw && typeof raw === "object" ? raw : {};
  const targetMode = readOption(source, "targetMode", "target_mode");
  const selectedPuuids = readOption(source, "selectedPuuids", "selected_puuids");
  const nameDisplayStrategy = readOption(source, "nameDisplayStrategy", "name_display_strategy");
  const normalized = {
    ...defaults,
    targetMode: VALID_TARGET_MODES.has(targetMode) ? targetMode : defaults.targetMode,
    selectedPuuids: Array.isArray(selectedPuuids) ? selectedPuuids.map(String).filter(Boolean).slice(0, 10) : [...defaults.selectedPuuids],
    nameDisplayStrategy: VALID_NAME_STRATEGIES.has(nameDisplayStrategy) ? nameDisplayStrategy : defaults.nameDisplayStrategy,
  };
  if (kind === "rating") {
    const showCurrentChampion = readOption(source, "showCurrentChampion", "show_current_champion");
    normalized.showCurrentChampion = typeof showCurrentChampion === "boolean" ? showCurrentChampion : defaults.showCurrentChampion;
    normalized.display = normalizeDisplay(defaults.display, source.display, RATING_DISPLAY_KEYS);
  } else if (kind === "jungle") {
    const showCurrentChampion = readOption(source, "showCurrentChampion", "show_current_champion");
    normalized.showCurrentChampion = typeof showCurrentChampion === "boolean" ? showCurrentChampion : defaults.showCurrentChampion;
    normalized.display = normalizeDisplay(defaults.display, source.display, JUNGLE_DISPLAY_KEYS);
  }
  return normalized;
}

export function getLeaguePresetOptions(settings, kind) {
  const raw = settings?.[presetOptionsKey(kind)] ?? settings?.[`in_game_${kind}_preset`];
  return normalizeLeaguePresetOptions(kind, raw);
}

/** Serialize the UI shape using the backend's canonical snake_case fields. */
export function serializeLeaguePresetOptions(kind, options) {
  const normalized = normalizeLeaguePresetOptions(kind, options);
  const result = {
    target_mode: normalized.targetMode,
    selected_puuids: [...normalized.selectedPuuids],
    name_display_strategy: normalized.nameDisplayStrategy,
  };
  if (kind === "rating" || kind === "jungle") {
    result.show_current_champion = normalized.showCurrentChampion;
    const aliases = kind === "rating" ? RATING_DISPLAY_KEYS : JUNGLE_DISPLAY_KEYS;
    result.display = Object.fromEntries(Object.keys(normalized.display).map((key) => [aliases[key] || key, Boolean(normalized.display[key])]));
  }
  return result;
}

export function playerKey(player, index = 0) {
  return String(player?.puuid || player?.playerPuuid || `player-${player?.team || "unknown"}-${index}`);
}

export function playerName(player, index, strategy = "preferName", streamerMode = false, useAliases = false) {
  const rawName = player?.summoner?.gameName || player?.game_name || `玩家 ${index + 1}`;
  const name = streamerMode ? maskLeagueName(rawName, index, useAliases, player?.puuid || player?.playerPuuid) : rawName;
  const champion = player?.champion_name || "";
  if (strategy === "preferChampionName") return champion || name;
  if (strategy === "championNameWithName") return champion ? `${champion} · ${name}` : name;
  return name;
}

export function selectLeaguePresetPlayers(players = [], options, ownPuuid = "") {
  const draft = normalizeLeaguePresetOptions("premade", options);
  const rows = Array.isArray(players) ? players : [];
  if (draft.targetMode === "selected") return rows.filter((player, index) => draft.selectedPuuids.includes(playerKey(player, index)));
  if (draft.targetMode === "all") return rows;
  const own = rows.find((player) => String(player?.puuid || player?.playerPuuid || "") === String(ownPuuid || ""));
  if (!own) return [];
  return rows.filter((player) => draft.targetMode === "friendly"
    ? String(player?.team) === String(own.team)
    : String(player?.team) !== String(own.team));
}

function finiteNumber(value) {
  if (value == null || value === "") return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function formatDecimal(value, digits = 2) {
  const number = finiteNumber(value);
  return number == null ? "—" : number.toFixed(digits);
}

function percent(value) {
  const number = finiteNumber(value);
  return number == null ? "—" : `${Math.round(number * 100)}%`;
}

function laneLabel(value) {
  return { top: "上半区", mid: "中路", bot: "下半区", unknown: "未知区域" }[value] || value || "未知区域";
}

function campLabel(value) {
  const [side, camp] = String(value || "").split(":");
  const sideText = { blue: "蓝色方野区", red: "红色方野区" }[side] || "未知野区";
  const campText = { blue: "蓝 BUFF", red: "红 BUFF", wolves: "三狼", raptors: "F6", krugs: "石甲虫" }[camp] || camp || "未知营地";
  return `${sideText} ${campText}`;
}

function clockLabel(seconds) {
  const value = finiteNumber(seconds);
  if (value == null) return "—";
  const rounded = Math.max(0, Math.round(value));
  return `${Math.floor(rounded / 60)}:${String(rounded % 60).padStart(2, "0")}`;
}

function ratingSummaryFallback(player) {
  if (player?.rating_summary) return player.rating_summary;
  const recent = player?.recent || {};
  const matches = finiteNumber(recent.matches);
  const usage = player?.champion_usage || {};
  return {
    win_rate: matches && matches > 0 ? finiteNumber(recent.wins) / matches : null,
    avg_kda: finiteNumber(usage.average_kda ?? recent.average_kda),
    main_champions: player?.champion_name ? [{ champion_name: player.champion_name }] : [],
    main_positions: [],
  };
}

export function buildRatingLines(players = [], options, privacy = {}) {
  const draft = normalizeLeaguePresetOptions("rating", options);
  return (Array.isArray(players) ? players : []).map((player, index) => {
    const summary = ratingSummaryFallback(player);
    const values = [];
    if (draft.display.winRate) values.push(`胜率 ${percent(summary.win_rate)}`);
    if (draft.display.kda) values.push(`KDA ${formatDecimal(summary.avg_kda)}`);
    if (draft.display.avgSoloKills && finiteNumber(summary.avg_solo_kills) != null) values.push(`场均单杀 ${formatDecimal(summary.avg_solo_kills, 1)}`);
    if (draft.display.avgVisionScore) values.push(`场均视野 ${formatDecimal(summary.avg_vision_score, 1)}`);
    if (draft.display.avgChampionDamage) values.push(`团队输出 ${percent(summary.avg_champion_damage_percentage_of_team)}`);
    if (draft.display.avgDamageTaken) values.push(`团队承伤 ${percent(summary.avg_damage_taken_percentage_of_team)}`);
    if (draft.display.avgGold) values.push(`团队经济 ${percent(summary.avg_gold_percentage_of_team)}`);
    if (draft.display.avgCsPerMinute) values.push(`补刀 ${formatDecimal(summary.avg_cs_per_minute, 1)}/分`);
    if (draft.display.avgKillParticipation) values.push(`参团 ${percent(summary.avg_kill_participation)}`);
    if (draft.display.avgDamageGoldEfficiency) values.push(`伤害/经济 ${formatDecimal(summary.avg_damage_gold_efficiency)}`);
    if (draft.display.mainChampions && Array.isArray(summary.main_champions) && summary.main_champions.length) values.push(`主力 ${summary.main_champions.map((row) => row.champion_name || row.champion_id).join("/")}`);
    if (draft.display.mainPositions && Array.isArray(summary.main_positions) && summary.main_positions.length) values.push(`主位置 ${summary.main_positions.map((row) => row.position).join("/")}`);
    if (draft.showCurrentChampion && player?.champion_name) values.push(`当前 ${player.champion_name}`);
    return `${playerName(player, index, draft.nameDisplayStrategy, privacy.streamerMode, privacy.useAliases)}：${values.length ? values.join("，") : "未选择可发送指标"}`;
  });
}

export function buildJungleLines(players = [], options, privacy = {}) {
  const draft = normalizeLeaguePresetOptions("jungle", options);
  return (Array.isArray(players) ? players : []).map((player, index) => {
    const jungle = player?.jungle_analysis || {};
    const name = playerName(player, index, draft.nameDisplayStrategy, privacy.streamerMode, privacy.useAliases);
    const values = [];
    if (!Number(jungle.games_analyzed || 0)) return `${name}：暂无可用打野时间线`;
    if (draft.display.activityPreference) values.push(`活动偏好 ${laneLabel(jungle.preferred_lane)}${jungle.zone_percentages ? `（上 ${percent(jungle.zone_percentages.top)} / 中 ${percent(jungle.zone_percentages.mid)} / 下 ${percent(jungle.zone_percentages.bot)}）` : ""}`);
    if (draft.display.firstClearDistribution) values.push(`首开 ${campLabel(jungle.preferred_start_camp)}`);
    if (draft.display.earlyGank) values.push(`早期参与 3 分钟 ${percent(jungle.early_gank?.level3_rate)} / 4 分钟 ${percent(jungle.early_gank?.level4_rate)}`);
    if (draft.display.dragonControl) values.push(`首龙 ${percent(jungle.objectives?.first_dragon_rate)}（${clockLabel(jungle.objectives?.avg_first_dragon_time_seconds)}），场均龙 ${formatDecimal(jungle.objectives?.avg_dragons, 1)}`);
    if (draft.display.monsterControl) values.push(`场均巢虫 ${formatDecimal(jungle.objectives?.avg_voidgrubs, 1)} / 先锋 ${formatDecimal(jungle.objectives?.avg_heralds, 1)} / 男爵 ${formatDecimal(jungle.objectives?.avg_barons, 1)}`);
    if (draft.display.mainChampions) {
      const champions = Array.isArray(jungle.main_champions) ? jungle.main_champions : [];
      values.push(champions.length ? `主力 ${champions.map((row) => row.champion_name || row.champion_id).join("/")}` : "暂无主力打野英雄");
    }
    if (draft.showCurrentChampion && player?.champion_name) values.push(`当前 ${player.champion_name}`);
    return `${name}：${values.length ? values.join("，") : "未选择可发送指标"}`;
  });
}

export function buildPremadeLines(players = [], options, privacy = {}) {
  const draft = normalizeLeaguePresetOptions("premade", options);
  const groups = new Map();
  (Array.isArray(players) ? players : []).forEach((player, index) => {
    if (!player?.premade_group) return;
    const key = Number(player.premade_group);
    if (!Number.isSafeInteger(key) || key <= 0) return;
    const names = groups.get(key) || [];
    names.push(playerName(player, index, draft.nameDisplayStrategy, privacy.streamerMode, privacy.useAliases));
    groups.set(key, names);
  });
  return [...groups.entries()]
    .filter(([, names]) => names.length > 1)
    .map(([group, names]) => `组排 ${String.fromCharCode(64 + group)}：${names.join("、")}`);
}

export function buildLeaguePresetLines(kind, players = [], options, privacy = {}) {
  if (kind === "rating") return buildRatingLines(players, options, privacy);
  if (kind === "jungle") return buildJungleLines(players, options, privacy);
  if (kind === "premade") return buildPremadeLines(players, options, privacy);
  return [];
}

// Compatibility wrappers used by the older toolkit composer. Keep their
// historical output stable while the shortcut manager uses the full shared
// generator above.
function displayName(player, index) {
  return player?.summoner?.gameName || player?.champion_name || `玩家${index + 1}`;
}

export function buildLeagueFormPreset(players = []) {
  return players.map((player, index) => {
    const recent = player?.recent || {};
    const matches = Number(recent.matches || 0);
    const winRate = matches ? Math.round(Number(recent.wins || 0) / matches * 100) : 0;
    const usage = player?.champion_usage || {};
    return `${displayName(player, index)}：近${matches}场 ${winRate}%胜率，${player?.champion_name || "当前英雄"} ${Number(usage.matches || 0)}场 / KDA ${Number(usage.average_kda || 0).toFixed(2)}`;
  });
}

export function buildLeaguePremadePreset(players = []) {
  return buildPremadeLines(players, { targetMode: "all", nameDisplayStrategy: "preferName" });
}

export function buildLeagueJunglePreset(players = []) {
  return players
    .map((player, index) => {
      const analysis = player?.jungle_analysis || {};
      if (!analysis.games_analyzed || !analysis.draft) return null;
      return `${displayName(player, index)}：${analysis.draft}`;
    })
    .filter(Boolean);
}
