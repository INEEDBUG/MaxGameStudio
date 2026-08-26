import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronUp, Clock3, Crown, Map as MapIcon } from "lucide-react";
import {
  getLeagueChampionIconUrl,
  getLeagueItemIconUrl,
  getLeaguePerkIconUrl,
  getLeagueSummonerSpellIconUrl,
} from "../../api/api";
import { fetchLeagueLoadoutCatalog, fetchLeagueMatchDetails } from "../../api/leagueLabApi";
import { maskLeagueName } from "../../utils/leagueStreamerMode";
import { formatLeagueTimestamp, leagueWinState } from "../../utils/leagueDisplay";
import LeagueMatchReplayActions from "./LeagueMatchReplayActions";

const TABS = [
  ["summary", "双方总览"],
  ["details", "详细属性"],
  ["runes", "符文"],
  ["events", "事件"],
  ["builds", "出装过程"],
  ["timeline", "时间线"],
];

// Keep every summary row on one shared column track.  The explicit minimum
// width lets narrow windows scroll instead of allowing KP/damage/CS to drift
// when a player name or item build is longer than its neighbour's.
const TEAM_TABLE_GRID = "grid min-w-[920px] grid-cols-[minmax(220px,1fr)_112px_82px_100px_106px_218px] gap-2";
const MATCH_METRIC_GRID = "grid min-w-0 flex-1 grid-cols-[minmax(104px,1fr)_minmax(104px,1fr)_minmax(104px,1fr)] gap-2";

function formatNumber(value) {
  if (value == null || value === "") return "—";
  const number = Number(value);
  return Number.isFinite(number) ? number.toLocaleString("zh-CN") : "—";
}

function formatDuration(value) {
  const seconds = Math.max(0, Number(value || 0));
  return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;
}

function assetId(value) {
  if (value && typeof value === "object") {
    value = value.item_id ?? value.itemId ?? value.perk_id ?? value.perkId ?? value.spell_id ?? value.spellId ?? value.id;
  }
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
}

function itemValues(value) {
  if (Array.isArray(value)) return value;
  if (!value || typeof value !== "object") return [];
  const indexed = Object.entries(value)
    .map(([key, item]) => {
      const match = String(key).match(/^(?:item|slot)?[_-]?(\d+)$/i);
      return match ? [Number(match[1]), item] : null;
    })
    .filter(Boolean)
    .sort((left, right) => left[0] - right[0]);
  return indexed.length ? indexed.map(([, item]) => item) : Object.values(value);
}

function itemSlots(player) {
  const sources = [player, player?.stats, player?.match_stats].filter((source) => source && typeof source === "object");
  let fallback = null;
  for (const source of sources) {
    for (const key of ["item_slots", "itemSlots", "items", "loadout"]) {
      const values = itemValues(source[key]);
      if (!values.length) continue;
      const slots = values.slice(0, 7).map(assetId);
      fallback ||= slots;
      if (slots.some(Boolean)) return [...slots, ...Array(7 - slots.length).fill(null)].slice(0, 7);
    }
    const slots = Array.from({ length: 7 }, (_, index) => assetId(
      source[`item${index}`] ?? source[`item_${index}`] ?? source[`Item${index}`],
    ));
    fallback ||= slots;
    if (slots.some(Boolean)) return slots;
  }
  return [...(fallback || []), ...Array(7 - (fallback || []).length).fill(null)].slice(0, 7);
}

function normalizedItems(player) {
  return itemSlots(player).filter(Boolean);
}

function canonicalName(player, index = 0) {
  const body = player || {};
  const name = String(body.game_name ?? body.gameName ?? body.riot_id_game_name ?? body.riotIdGameName ?? body.name ?? body.champion_name ?? `玩家 ${index + 1}`).trim();
  const tag = String(body.tag_line ?? body.tagLine ?? body.riot_id_tagline ?? body.riotIdTagline ?? "").trim();
  if (!tag || name.endsWith(`#${tag}`) || name.includes(` #${tag}`)) return name;
  return `${name}#${tag}`;
}

function playerName(player, index, streamerMode, useAliases) {
  const raw = canonicalName(player, index);
  if (streamerMode) {
    // LeagueAkari's privacy mode intentionally replaces Riot IDs with the
    // champion name in the match card. This also avoids repeating the same
    // masked name in the compact two-team roster.
    return player?.champion_name || maskLeagueName(raw, index, useAliases, player?.puuid);
  }
  return raw;
}

function Icon({ src, title, className = "h-7 w-7" }) {
  const [failed, setFailed] = useState(false);
  if (!src || failed) {
    return <span aria-label={title || "资源"} title={title} className={`${className} inline-grid place-items-center rounded-md border border-white/10 bg-black/20 text-[9px] text-cs2-text-muted`}>—</span>;
  }
  return <img src={src} alt="" title={title} onError={() => setFailed(true)} className={`${className} rounded-md border border-white/10 bg-black/20 object-cover`} />;
}

function firstAssetId(values) {
  for (const value of values) {
    const id = assetId(value);
    if (id) return id;
  }
  return null;
}

function spellIds(player) {
  const sources = [player, player?.stats, player?.match_stats].filter((source) => source && typeof source === "object");
  for (const source of sources) {
    const array = source.spells ?? source.summonerSpells ?? source.summoner_spells;
    if (Array.isArray(array)) {
      const ids = array.map(assetId).filter(Boolean).slice(0, 2);
      if (ids.length) return ids;
    }
    const ids = [
      firstAssetId([source.spell1_id, source.spell1Id, source.summoner1Id, source.summoner_spell1_id]),
      firstAssetId([source.spell2_id, source.spell2Id, source.summoner2Id, source.summoner_spell2_id]),
    ].filter(Boolean);
    if (ids.length) return ids;
  }
  return [];
}

function SpellIcons({ player, compact = false }) {
  const ids = spellIds(player);
  if (!ids.length) return null;
  return <div className="flex gap-1">
    {ids.map((id) => <Icon key={id} src={getLeagueSummonerSpellIconUrl(id)} title={`召唤师技能 ${id}`} className={compact ? "h-6 w-6" : "h-8 w-8"} />)}
  </div>;
}

function Loadout({ player, compact = false }) {
  const items = normalizedItems(player);
  return <div className="flex min-w-0 flex-wrap items-center gap-1">
    {items.slice(0, 7).map((id, index) => <Icon key={`${id}-${index}`} src={getLeagueItemIconUrl(id)} title={`装备 ${id}`} className={compact ? "h-5 w-5" : "h-7 w-7"} />)}
    {!items.length ? <span className="text-[10px] text-cs2-text-muted">无装备记录</span> : null}
  </div>;
}

function uniqueParticipants(match) {
  const rows = Array.isArray(match?.participants) ? match.participants : [];
  const seen = new Set();
  const unique = [];
  for (const row of rows) {
    if (!row || typeof row !== "object") continue;
    const key = row.puuid || row.participant_puuid || row.participant_id || row.participantId || `${row.team_id || row.teamId}:${row.champion_id || row.championId}:${unique.length}`;
    if (seen.has(String(key))) continue;
    seen.add(String(key));
    unique.push({
      ...row,
      participant_id: row.participant_id ?? row.participantId,
      puuid: row.puuid ?? row.participant_puuid,
      team_id: row.team_id ?? row.teamId,
      champion_id: row.champion_id ?? row.championId,
      champion_name: row.champion_name ?? row.championName,
      game_name: row.game_name ?? row.gameName ?? row.riotIdGameName,
      tag_line: row.tag_line ?? row.tagLine ?? row.riotIdTagline,
      spell1_id: row.spell1_id ?? row.spell1Id ?? row.summoner1Id ?? row.stats?.spell1Id ?? row.stats?.summoner1Id,
      spell2_id: row.spell2_id ?? row.spell2Id ?? row.summoner2Id ?? row.stats?.spell2Id ?? row.stats?.summoner2Id,
      spells: row.spells ?? row.summonerSpells ?? row.summoner_spells,
      items: normalizedItems(row),
      item_slots: itemSlots(row),
      perks: row.perks ?? row.runes ?? [],
      damage: row.damage ?? row.totalDamageDealtToChampions,
      damage_taken: row.damage_taken ?? row.totalDamageTaken,
      cs: row.cs ?? ((Number(row.totalMinionsKilled) || 0) + (Number(row.neutralMinionsKilled) || 0)),
    });
  }
  if (unique.length) return unique;
  if (match && (match.champion_id || match.championId || match.participant_puuid)) {
    return [{
      ...match,
      puuid: match.participant_puuid,
      team_id: match.team_id ?? match.teamId,
      champion_id: match.champion_id ?? match.championId,
      champion_name: match.champion_name ?? match.championName,
      game_name: match.game_name ?? match.gameName,
      items: normalizedItems(match),
      item_slots: itemSlots(match),
    }];
  }
  return [];
}

/** Build the read-only historical payload consumed by LeagueOngoingGame. */
export function buildLeagueHistoricalPreview(match, participantRows = null) {
  const participants = Array.isArray(participantRows) ? participantRows : uniqueParticipants(match);
  const players = participants.map((player, index) => ({
    participant_id: player?.participant_id ?? player?.participantId ?? index + 1,
    puuid: player?.puuid || player?.participant_puuid || "",
    team: player?.team_id ?? player?.teamId ?? "UNKNOWN",
    champion_id: Number(player?.champion_id ?? player?.championId ?? 0),
    champion_name: player?.champion_name || player?.championName || "未知英雄",
    position: player?.position || player?.role || "",
    summoner: {
      gameName: player?.game_name || player?.gameName || player?.riot_id_game_name || "",
      tagLine: player?.tag_line || player?.tagLine || player?.riot_id_tagline || "",
      profileIconId: player?.profile_icon_id ?? player?.profileIconId,
    },
    tag: player?.tag || null,
    premade_group: null,
    recent: { matches: 0, wins: 0 },
    champion_usage: { matches: 0, wins: 0, average_kda: 0 },
    match_stats: {
      kills: Number(player?.kills || 0),
      deaths: Number(player?.deaths || 0),
      assists: Number(player?.assists || 0),
      kda: (Number(player?.kills || 0) + Number(player?.assists || 0)) / Math.max(1, Number(player?.deaths || 0)),
      damage: Number(player?.damage ?? player?.totalDamageDealtToChampions ?? 0),
      gold: Number(player?.gold ?? player?.goldEarned ?? 0),
      cs: Number(player?.cs ?? 0),
      items: normalizedItems(player),
      win: player?.win ?? null,
    },
  }));
  const teamIds = [...new Set(players.map((player) => player.team))].filter((team) => team !== "UNKNOWN");
  return {
    phase: "HistoricalPreview",
    queue: { id: match?.queue_id ?? match?.queueId, gameMode: match?.game_mode || match?.gameMode },
    game_id: match?.game_id ?? match?.gameId,
    players,
    available: players.length > 0,
    historical_preview: true,
    source: match?.source || "lcu",
    metadata: {
      game_id: match?.game_id ?? match?.gameId,
      played_at: match?.played_at ?? match?.playedAt,
      duration_seconds: match?.duration_seconds ?? match?.durationSeconds,
      game_mode: match?.game_mode ?? match?.gameMode,
      game_type: match?.game_type ?? match?.gameType,
      queue_id: match?.queue_id ?? match?.queueId,
    },
    teams: teamIds.map((teamId) => ({ team_id: teamId, players: players.filter((player) => player.team === teamId) })),
  };
}

function mapName(match) {
  const id = Number(match?.map_id ?? match?.mapId);
  return ({ 11: "召唤师峡谷", 12: "嚎哭深渊", 21: "水晶之痕" })[id] || match?.map_name || match?.game_mode || "未知地图";
}

function queueName(match) {
  const id = Number(match?.queue_id ?? match?.queueId);
  return ({ 400: "普通对战", 420: "单双排位", 430: "匹配对战", 440: "灵活排位", 450: "极地大乱斗", 490: "快速游戏" })[id] || match?.game_mode || match?.game_type || "未知队列";
}

function relativeTime(value) {
  const raw = value == null ? 0 : (typeof value === "number" || /^\d+(\.\d+)?$/.test(String(value)) ? Number(value) : Date.parse(value));
  const timestamp = Number.isFinite(raw) ? (raw > 0 && raw < 1e12 ? raw * 1000 : raw) : 0;
  if (!timestamp) return "时间未知";
  const diff = Math.max(0, Date.now() - timestamp);
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days} 天前`;
  return formatLeagueTimestamp(value);
}

function TeamTable({ players, targetPuuid, streamerMode, useAliases, onOpenPlayer }) {
  const hasTeamContext = players.length > 1;
  const teamKills = players.reduce((sum, player) => sum + Number(player.kills || 0), 0);
  const teamDamage = players.reduce((sum, player) => sum + Number(player.damage || player.totalDamageDealtToChampions || 0), 0);
  return <div data-testid="league-team-table" className="overflow-x-auto rounded-xl border border-cs2-border-subtle">
    <div className={`${TEAM_TABLE_GRID} border-b border-cs2-border-subtle bg-white/[.035] px-3 py-2 text-[10px] font-semibold uppercase tracking-wide text-cs2-text-muted`}>
      <span>玩家</span><span className="text-center">K / D / A</span><span className="text-right">参团</span><span className="text-right">伤害</span><span className="text-right">补刀 / 金币</span><span>装备</span>
    </div>
    {players.map((player, index) => {
      const highlighted = player.puuid && player.puuid === targetPuuid;
      const kp = hasTeamContext && teamKills ? Math.round((Number(player.kills || 0) + Number(player.assists || 0)) / teamKills * 100) : null;
      const damageShare = hasTeamContext && teamDamage ? Math.round(Number(player.damage || 0) / teamDamage * 100) : null;
      return <div key={player.puuid || player.participant_id || index} className={`${TEAM_TABLE_GRID} items-center border-b border-cs2-border-subtle px-3 py-2 text-xs tabular-nums last:border-b-0 ${highlighted ? "bg-cyan-400/[.08]" : ""}`}>
        <button type="button" disabled={!player.puuid} onClick={() => player.puuid && onOpenPlayer?.(player.puuid)} className="flex min-w-0 items-center gap-2 text-left disabled:cursor-default">
          <Icon src={getLeagueChampionIconUrl(player.champion_id)} title={player.champion_name} className="h-8 w-8" />
          <span className="min-w-0"><b className="block truncate">{playerName(player, index, streamerMode, useAliases)}</b><span className="text-[10px] text-cs2-text-muted">{player.position || player.role || player.champion_name}</span></span>
        </button>
        <span className="text-center font-mono font-bold"><span>{player.kills ?? "—"}</span><span className="text-cs2-text-muted"> / </span><span className="text-rose-300">{player.deaths ?? "—"}</span><span className="text-cs2-text-muted"> / </span><span>{player.assists ?? "—"}</span></span>
        <span className="text-right font-mono">{kp == null ? "—" : `${kp}%`}</span>
        <span className="text-right"><b>{formatNumber(player.damage ?? player.totalDamageDealtToChampions)}</b><small className="block text-[9px] text-cs2-text-muted">{damageShare == null ? "—" : `${damageShare}%`}</small></span>
        <span className="text-right"><b>{formatNumber(player.cs)}</b><small className="block text-[9px] text-cs2-text-muted">{formatNumber(player.gold)}g</small></span>
        <Loadout player={player} compact />
      </div>;
    })}
  </div>;
}

function Stat({ label, value, icon: IconComponent }) {
  return <span className="rounded-xl border border-cs2-border-subtle bg-black/10 p-3 text-xs text-cs2-text-muted">{IconComponent ? <IconComponent className="mr-1 inline h-3.5 w-3.5" /> : null}{label}<b className="mt-1 block text-base text-cs2-text-primary">{value}</b></span>;
}

function participantDisplay(player, index, streamerMode, useAliases) {
  return playerName(player || {}, index, streamerMode, useAliases);
}

const RAW_STAT_LABELS = {
  kills: "击杀", deaths: "死亡", assists: "助攻", win: "胜利", champLevel: "英雄等级",
  goldEarned: "获得金币", goldSpent: "消费金币", totalMinionsKilled: "线上补刀",
  neutralMinionsKilled: "野怪补刀", totalDamageDealtToChampions: "对英雄伤害",
  totalDamageTaken: "承受伤害", totalHeal: "治疗量", totalTimeCCDealt: "控制时长",
  damageDealtToTurrets: "防御塔伤害", visionScore: "视野得分", wardsPlaced: "插眼",
  wardsKilled: "排眼", largestKillingSpree: "最大连杀", doubleKills: "双杀",
  tripleKills: "三杀", quadraKills: "四杀", pentaKills: "五杀",
};

// Adapted from LeagueAkari's MIT-licensed match detail taxonomy. Keep the
// client-facing matrix readable while still preserving unknown Riot fields.
const RAW_STAT_GROUPS = [
  ["combat", "战斗数据", /^(kills|deaths|assists|firstBlood|doubleKills|tripleKills|quadraKills|pentaKills|unrealKills|killingSprees|largestKillingSpree|largestMultiKill|longestTimeSpentLiving|largestCriticalStrike|legendaryCount|highestChampionDamage|damageGoldEfficiency)$/],
  ["damage", "伤害与承伤", /(Damage|damage|Shielded|shielded|Mitigated|mitigated)/],
  ["control", "控制", /(CC|CrowdControl|Immobil|immobil|knockEnemy)/],
  ["vision", "视野", /(vision|Vision|ward|Ward|sweeper|Sweeper)/],
  ["buildings", "建筑与防御塔", /(turret|Turret|tower|Tower|inhibitor|Inhibitor|nexus|Nexus|Plate|plate)/],
  ["economy", "经济与补刀", /(gold|Gold|minion|Minion|cs|Cs|purchased|Purchased|supportQuest|Mejais|mejais)/],
  ["healing", "治疗与护盾", /(heal|Heal|shield|Shield)/],
  ["pings", "信号", /Pings$/],
  ["objectives", "野区与地图目标", /(baron|Baron|dragon|Dragon|riftHerald|RiftHerald|epicMonster|EpicMonster|jungle|Jungle|scuttle|Scuttle|buffsStolen|voidMonster)/],
  ["abilities", "技能使用", /(abilityUses|spell\d|summoner\d|skillshots|SkillShots|snowballsHit)/],
  ["survival", "生存能力", /(surviv|Surviv|deathsByEnemy|saveAlly|quickCleanse|blastCone|fistBump)/],
  ["teamfight", "团战", /(Ace|ace|fullTeamTakedown|AssistStreak|teleportTakedowns)/],
  ["state", "比赛状态", /^(gameEnded|caused|earlySurrender|teamEarly|teamIGNB|wasPremade|wasSevere|hadAfk|PlayerBehavior|positionAssigned|selectedRole|playedChampSelect)/],
  ["misc", "其他", /.*/],
];

const RAW_STAT_HIDDEN = /^(identity|champion(Id|Name|Transform)|participantId|teamId|puuid|summoner(Id|Name|Level)|profileIcon|riotId|lane$|role$|teamPosition|individualPosition|placement|subteamPlacement|playerSubteamId|item[0-6]|roleBoundItem|playerAugment|perks$|perk\d|perkPrimaryStyle|perkSubStyle|PlayerScore|playerScore|combatPlayerScore|objectivePlayerScore|totalPlayerScore|totalScoreRank|win$|eligibleForProgression|InfernalScalePickup|SWARM_|poroExplosions|spell[12]Id|legendaryItemUsed)/;

function rawStatGroup(key) {
  return RAW_STAT_GROUPS.find(([, , matcher]) => matcher.test(key))?.[0] || "misc";
}

function rawValue(value) {
  if (typeof value === "boolean") return value ? "是" : "否";
  if (typeof value === "number") return Number.isInteger(value) ? formatNumber(value) : value.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
  return value == null || value === "" ? "—" : String(value);
}

function RawStatHeader({ statKey, participants, streamerMode, useAliases }) {
  const cells = participants.map((player, index) => ({
    index,
    player,
    value: player.raw_stats?.[statKey],
  }));
  const numeric = cells.filter((cell) => typeof cell.value === "number" && Number.isFinite(cell.value));
  const label = RAW_STAT_LABELS[statKey] || statKey;
  if (!numeric.length) return <span className="block truncate" title={statKey}>{label}</span>;
  const values = numeric.map((cell) => Number(cell.value));
  const min = Math.min(0, ...values);
  const max = Math.max(0, ...values);
  const span = Math.max(1, max - min);
  return <details className="group relative">
    <summary className="cursor-pointer list-none truncate text-cyan-100" title={`${statKey} · 点击比较十名玩家`}>{label}</summary>
    <div className="absolute left-full top-0 z-50 ml-2 w-64 rounded-xl border border-cyan-400/25 bg-cs2-bg-elevated p-3 text-left shadow-2xl">
      <b className="block truncate text-[11px] text-cs2-text-primary">{label}</b>
      <small className="mb-2 block truncate font-normal text-cs2-text-muted">{statKey}</small>
      <div className="space-y-1.5">{numeric.map(({ player, index, value }) => {
        const width = Math.max(3, ((Number(value) - min) / span) * 100);
        return <div key={player.puuid || player.participant_id || index} className="grid grid-cols-[72px_1fr_auto] items-center gap-2 font-normal">
          <span className="truncate text-[9px] text-cs2-text-muted">{participantDisplay(player, index, streamerMode, useAliases)}</span>
          <span className="h-1.5 overflow-hidden rounded-full bg-white/5"><i className="block h-full rounded-full bg-cyan-400" style={{ width: `${width}%` }}/></span>
          <span className="font-mono text-[9px] text-cs2-text-primary">{rawValue(value)}</span>
        </div>;
      })}</div>
    </div>
  </details>;
}

function RawDetailsTab({ match, participants, streamerMode, useAliases }) {
  const [filter, setFilter] = useState("");
  const [groupFilter, setGroupFilter] = useState("all");
  const groups = useMemo(() => {
    const all = new Set();
    participants.forEach((player) => Object.keys(player.raw_stats || {}).forEach((key) => all.add(key)));
    const needle = filter.trim().toLowerCase();
    const grouped = new Map();
    [...all]
      .filter((key) => !RAW_STAT_HIDDEN.test(key))
      .filter((key) => !needle || key.toLowerCase().includes(needle) || String(RAW_STAT_LABELS[key] || "").toLowerCase().includes(needle))
      .forEach((key) => {
        const group = rawStatGroup(key);
        if (groupFilter !== "all" && group !== groupFilter) return;
        if (!grouped.has(group)) grouped.set(group, []);
        grouped.get(group).push(key);
      });
    return RAW_STAT_GROUPS
      .map(([id, label]) => ({ id, label, keys: (grouped.get(id) || []).sort((a, b) => a.localeCompare(b)) }))
      .filter((group) => group.keys.length);
  }, [filter, groupFilter, participants]);
  const keyCount = groups.reduce((sum, group) => sum + group.keys.length, 0);
  return <section className="space-y-2">
    <div className="flex flex-wrap items-center gap-3 rounded-xl border border-cs2-border-subtle bg-white/[.025] p-2 text-[10px] text-cs2-text-muted">
      <span>Game ID <b className="select-text text-cs2-text-primary">{match.game_id || "—"}</b></span>
      <span>数据源 <b className="text-cs2-text-primary">{String(match.source || "LCU").toUpperCase()}</b></span>
      <span>版本 <b className="text-cs2-text-primary">{match.game_version || "—"}</b></span>
      <span>地图 <b className="text-cs2-text-primary">{match.map_id || "—"}</b></span>
      <span><b className="text-cs2-text-primary">{keyCount}</b> 项</span>
      <select aria-label="属性分组" value={groupFilter} onChange={(event) => setGroupFilter(event.target.value)} className="ml-auto rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs text-cs2-text-primary"><option value="all">全部分组</option>{RAW_STAT_GROUPS.map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select>
      <input value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="筛选属性名称…" className="min-w-52 rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-1.5 text-xs text-cs2-text-primary outline-none focus:border-cyan-400/60"/>
    </div>
    <div className="max-h-[430px] overflow-auto rounded-xl border border-cs2-border-subtle">
      {groups.length ? <table className="min-w-max border-collapse text-[10px]"><thead className="sticky top-0 z-20 bg-cs2-bg-elevated"><tr><th className="sticky left-0 z-30 min-w-40 border-b border-r border-cs2-border-subtle bg-cs2-bg-elevated p-2 text-left">属性</th>{participants.map((player, index) => <th key={player.puuid || player.participant_id || index} className="min-w-24 border-b border-cs2-border-subtle p-2"><Icon src={getLeagueChampionIconUrl(player.champion_id)} title={player.champion_name} className="mx-auto h-7 w-7"/><span className="mt-1 block max-w-24 truncate">{participantDisplay(player, index, streamerMode, useAliases)}</span></th>)}</tr></thead><tbody>{groups.flatMap((group) => [<tr key={`group-${group.id}`}><th colSpan={participants.length + 1} className="sticky left-0 z-[9] border-y border-cyan-400/15 bg-cyan-400/[.07] px-3 py-2 text-left text-[10px] font-black uppercase tracking-[.16em] text-cyan-200">{group.label}<span className="ml-2 font-mono font-normal text-cs2-text-muted">{group.keys.length}</span></th></tr>, ...group.keys.map((key) => <tr key={key} className="odd:bg-white/[.018]"><th className="sticky left-0 z-10 max-w-48 border-r border-t border-cs2-border-subtle bg-cs2-bg-elevated p-2 text-left"><RawStatHeader statKey={key} participants={participants} streamerMode={streamerMode} useAliases={useAliases}/>{RAW_STAT_LABELS[key] ? <small className="block truncate font-normal text-cs2-text-muted">{key}</small> : null}</th>{participants.map((player, index) => <td key={player.puuid || player.participant_id || index} className="max-w-32 truncate border-t border-cs2-border-subtle p-2 text-center font-mono" title={rawValue(player.raw_stats?.[key])}>{rawValue(player.raw_stats?.[key])}</td>)}</tr>)])}</tbody></table> : <p className="py-12 text-center text-xs text-cs2-text-muted">没有匹配的属性</p>}
    </div>
  </section>;
}

function plainDescription(value) {
  return String(value || "").replace(/<br\s*\/?\s*>/gi, " ").replace(/<[^>]+>/g, "").replace(/&nbsp;/g, " ").replace(/&amp;/g, "&").trim();
}

function RunesTab({ participants, perkMap, streamerMode, useAliases }) {
  return <div className="space-y-2">{participants.map((player, index) => <section key={player.puuid || player.participant_id || index} className="flex flex-wrap items-center gap-3 rounded-xl border border-cs2-border-subtle p-3">
    <Icon src={getLeagueChampionIconUrl(player.champion_id)} title={player.champion_name} className="h-9 w-9"/>
    <span className="min-w-[150px] flex-1 text-xs"><b className="block">{participantDisplay(player, index, streamerMode, useAliases)}</b><small className="text-cs2-text-muted">{player.position || player.role || player.champion_name}</small></span>
    <SpellIcons player={player}/>
    <div className="flex max-w-xl flex-1 flex-wrap gap-2">{(player.perks || []).map(assetId).filter(Boolean).length ? (player.perks || []).map(assetId).filter(Boolean).map((perkId) => { const perk = perkMap.get(Number(perkId)); const description = plainDescription(perk?.long_description || perk?.short_description); return <span key={perkId} className="flex max-w-56 items-center gap-1.5 rounded-lg bg-white/[.035] px-2 py-1"><Icon src={getLeaguePerkIconUrl(perkId)} title={perk?.name || `符文 ${perkId}`} className="h-7 w-7"/><span className="min-w-0 text-[10px]"><b className="block truncate text-cs2-text-primary">{perk?.name || `符文 ${perkId}`}</b>{description ? <small className="block max-w-44 truncate text-cs2-text-muted" title={description}>{description}</small> : null}</span></span>; }) : <span className="text-[11px] text-cs2-text-muted">无符文记录</span>}{player.augments?.length ? <span className="w-full text-[10px] text-violet-200">强化：{player.augments.join(" · ")}</span> : null}</div>
  </section>)}</div>;
}

const EVENT_LABELS = {
  CHAMPION_KILL: "英雄击杀",
  ELITE_MONSTER_KILL: "史诗野怪",
  BUILDING_KILL: "建筑摧毁",
  TURRET_PLATE_DESTROYED: "防御塔镀层",
  ITEM_PURCHASED: "购买装备",
  ITEM_SOLD: "出售装备",
  ITEM_UNDO: "撤销购买",
  SKILL_LEVEL_UP: "技能升级",
};

const MAP_DOMAINS = {
  11: { minX: 0, minY: 0, maxX: 14820, maxY: 14881 },
  12: { minX: -28, minY: -19, maxX: 12849, maxY: 12858 },
  21: { minX: 0, minY: 0, maxX: 15000, maxY: 15000 },
};

function MapPositionPreview({ mapId, position }) {
  const domain = MAP_DOMAINS[Number(mapId)];
  if (!domain || !position || !Number.isFinite(Number(position.x)) || !Number.isFinite(Number(position.y))) return null;
  const left = Math.max(0, Math.min(100, (Number(position.x) - domain.minX) / (domain.maxX - domain.minX) * 100));
  const top = Math.max(0, Math.min(100, (domain.maxY - Number(position.y)) / (domain.maxY - domain.minY) * 100));
  return <details className="mt-1 w-fit"><summary className="cursor-pointer text-[10px] font-semibold text-cyan-300">查看地图位置</summary><div className="relative mt-1 h-40 w-40 overflow-hidden rounded-lg border border-cs2-border-subtle bg-[linear-gradient(45deg,rgba(34,211,238,.035)_25%,transparent_25%,transparent_75%,rgba(34,211,238,.035)_75%),linear-gradient(45deg,rgba(34,211,238,.035)_25%,transparent_25%,transparent_75%,rgba(34,211,238,.035)_75%)] bg-[length:24px_24px] bg-[position:0_0,12px_12px]"><span className="absolute left-2 top-2 text-[9px] font-bold text-cs2-text-muted">MAP {mapId}</span><i className="absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white bg-cyan-400 shadow-[0_0_12px_rgba(34,211,238,.9)]" style={{ left: `${left}%`, top: `${top}%` }}/></div></details>;
}

function eventActorId(event) {
  return Number(event.killerId || event.participantId || event.creatorId || 0);
}

function EventsTab({ details, participants, mapId, streamerMode, useAliases }) {
  const majorTypes = ["CHAMPION_KILL", "ELITE_MONSTER_KILL", "BUILDING_KILL", "TURRET_PLATE_DESTROYED"];
  const [selected, setSelected] = useState(majorTypes);
  const participantIds = participants.map((player) => Number(player.participant_id)).filter(Boolean);
  const [selectedParticipants, setSelectedParticipants] = useState(participantIds);
  const byId = new Map(participants.map((player) => [Number(player.participant_id), player]));
  const toggle = (type) => setSelected((current) => current.includes(type) ? current.filter((value) => value !== type) : [...current, type]);
  const toggleParticipant = (id) => setSelectedParticipants((current) => current.includes(id) ? current.filter((value) => value !== id) : [...current, id]);
  const rows = (details?.events || []).filter((event) => {
    if (!selected.includes(event.type)) return false;
    const assists = Array.isArray(event.assistingParticipantIds) ? event.assistingParticipantIds.map(Number) : [];
    const involved = [eventActorId(event), Number(event.victimId || 0), ...assists].filter(Boolean);
    return !involved.length || involved.some((id) => selectedParticipants.includes(id));
  });
  const plateCounts = (details?.events || []).filter((event) => event.type === "TURRET_PLATE_DESTROYED").reduce((counts, event) => {
    const id = eventActorId(event);
    if (id) counts.set(id, (counts.get(id) || 0) + 1);
    return counts;
  }, new Map());
  return <div className="grid gap-3 lg:grid-cols-[1fr_190px]">
    <div className="max-h-[430px] overflow-y-auto rounded-xl border border-cs2-border-subtle p-3">{rows.length ? <ol className="space-y-3">{rows.map((event, index) => { const actor = byId.get(eventActorId(event)); const victim = byId.get(Number(event.victimId || 0)); return <li key={`${event.timestamp}-${event.type}-${index}`} className="relative border-l border-cyan-400/25 pl-4 text-xs before:absolute before:-left-1 before:top-1 before:h-2 before:w-2 before:rounded-full before:bg-cyan-300"><span className="font-mono text-[10px] text-cs2-text-muted">{formatDuration(Number(event.timestamp || 0) / 1000)}</span><b className="ml-2">{EVENT_LABELS[event.type] || event.type}</b><p className="mt-1 text-cs2-text-secondary">{actor ? participantDisplay(actor, eventActorId(event), streamerMode, useAliases) : "系统"}{victim ? ` → ${participantDisplay(victim, Number(event.victimId), streamerMode, useAliases)}` : ""}{event.monsterType ? ` · ${event.monsterType}` : ""}{event.buildingType ? ` · ${event.laneType || ""} ${event.buildingType}` : ""}</p><MapPositionPreview mapId={mapId} position={event.position}/></li>;})}</ol> : <p className="py-8 text-center text-cs2-text-muted">当前筛选下没有事件</p>}</div>
    <aside className="max-h-[430px] overflow-y-auto rounded-xl border border-cs2-border-subtle p-3"><h4 className="mb-2 text-xs font-bold">事件筛选</h4>{majorTypes.map((type) => <label key={type} className="flex items-center gap-2 py-1.5 text-xs"><input type="checkbox" checked={selected.includes(type)} onChange={() => toggle(type)} className="accent-cyan-400"/>{EVENT_LABELS[type]}</label>)}<div className="my-2 border-t border-cs2-border-subtle"/><h4 className="mb-1 text-xs font-bold">按英雄筛选</h4>{participants.map((player, index) => { const id = Number(player.participant_id); return <label key={id || index} className="flex items-center gap-2 py-1 text-[10px]"><input type="checkbox" checked={selectedParticipants.includes(id)} onChange={() => toggleParticipant(id)} className="accent-cyan-400"/><Icon src={getLeagueChampionIconUrl(player.champion_id)} title={player.champion_name} className="h-5 w-5"/><span className="min-w-0 flex-1 truncate">{participantDisplay(player, index, streamerMode, useAliases)}</span>{plateCounts.get(id) ? <b title="镀层数" className="text-amber-300">{plateCounts.get(id)}</b> : null}</label>;})}</aside>
  </div>;
}

const ANVIL_ITEM_IDS = new Set([6032, 220000]);

function itemTimelineWithSpacers(events) {
  const rows = [];
  let lastPurchase = 0;
  for (const event of events) {
    const timestamp = Number(event.timestamp || 0);
    if (event.type === "ITEM_PURCHASED" && lastPurchase && timestamp - lastPurchase > 30000) {
      rows.push({ type: "SPACER", timestamp, key: `spacer-${timestamp}-${rows.length}` });
    }
    rows.push({ ...event, key: `${event.type}-${timestamp}-${rows.length}` });
    if (event.type === "ITEM_PURCHASED") lastPurchase = timestamp;
  }
  return rows;
}

function BuildsTab({ details, participants, streamerMode, useAliases }) {
  const byParticipant = new Map();
  for (const event of details?.events || []) {
    if (!["ITEM_PURCHASED", "ITEM_SOLD", "ITEM_UNDO", "SKILL_LEVEL_UP"].includes(event.type)) continue;
    const id = Number(event.participantId || 0);
    if (!byParticipant.has(id)) byParticipant.set(id, []);
    byParticipant.get(id).push(event);
  }
  const [selectedParticipant, setSelectedParticipant] = useState("all");
  const visiblePlayers = selectedParticipant === "all" ? participants : participants.filter((player) => String(player.participant_id) === selectedParticipant);
  return <div className="grid gap-3 lg:grid-cols-[1fr_170px]">
    <div className="max-h-[460px] space-y-2 overflow-y-auto pr-1">{visiblePlayers.map((player, index) => {
      const events = byParticipant.get(Number(player.participant_id)) || [];
      const itemEvents = events.filter((event) => ["ITEM_PURCHASED", "ITEM_SOLD", "ITEM_UNDO"].includes(event.type));
      const skills = events.filter((event) => event.type === "SKILL_LEVEL_UP");
      const displaySkills = skills.reduce((rows, event) => {
        const level = event.levelUpType === "EVOLVE" ? null : rows.filter((row) => row.level != null).length + 1;
        rows.push({ ...event, level });
        return rows;
      }, []);
      const itemTimeline = itemTimelineWithSpacers(itemEvents);
      const anvilCount = itemEvents.filter((event) => event.type === "ITEM_PURCHASED" && ANVIL_ITEM_IDS.has(Number(event.itemId))).length;
      return <section key={player.puuid || player.participant_id || index} className="rounded-xl border border-cs2-border-subtle p-3"><div className="mb-3 flex items-center gap-2"><Icon src={getLeagueChampionIconUrl(player.champion_id)} title={player.champion_name} className="h-8 w-8"/><b className="text-xs">{participantDisplay(player, index, streamerMode, useAliases)}</b>{anvilCount ? <span className="rounded bg-amber-400/10 px-2 py-1 text-[9px] font-bold text-amber-200">铁砧 × {anvilCount}</span> : null}</div><div className="grid gap-3 md:grid-cols-2"><div><h5 className="mb-2 text-[10px] font-semibold text-cs2-text-muted">技能升级顺序</h5><div className="flex flex-wrap gap-1">{displaySkills.length ? displaySkills.map((event, skillIndex) => <span key={`${event.timestamp}-${skillIndex}`} title={`${event.level ? `${event.level}级 · ` : ""}${formatDuration(Number(event.timestamp || 0) / 1000)}${event.levelUpType ? ` · ${event.levelUpType}` : ""}`} className={`relative grid h-7 w-7 place-items-center rounded text-xs font-black ${event.levelUpType === "EVOLVE" ? "border border-amber-300 bg-rose-400/30 text-amber-100" : "bg-violet-400/10 text-violet-200"}`}>{["?", "Q", "W", "E", "R"][Number(event.skillSlot || 0)] || "?"}{event.level ? <small className="absolute -bottom-1 -right-1 min-w-3 rounded bg-black/70 px-0.5 text-[8px] leading-3 text-white">{event.level}</small> : null}</span>) : <span className="text-[10px] text-cs2-text-muted">无数据</span>}</div></div><div><h5 className="mb-2 text-[10px] font-semibold text-cs2-text-muted">装备操作时间线</h5><div className="flex flex-wrap items-start gap-1">{itemTimeline.length ? itemTimeline.map((event) => { if (event.type === "SPACER") return <span key={event.key} aria-label="购买阶段分隔" className="grid h-8 w-7 place-items-center text-sm text-cs2-text-muted">→</span>; const itemId = Number(event.itemId || event.afterId || event.beforeId || 0); const label = event.type === "ITEM_SOLD" ? "出售" : event.type === "ITEM_UNDO" ? "撤销" : "购买"; return <span key={event.key} className={`rounded p-0.5 text-center ${event.type === "ITEM_SOLD" ? "bg-rose-400/10" : event.type === "ITEM_UNDO" ? "bg-amber-400/10" : ""}`}>{itemId ? <Icon src={getLeagueItemIconUrl(itemId)} title={`${formatDuration(Number(event.timestamp || 0) / 1000)} · ${label}装备 ${itemId}`} className="h-8 w-8"/> : <span className="grid h-8 w-8 place-items-center rounded bg-white/5 text-[9px]">{label}</span>}<small className="block font-mono text-[8px] text-cs2-text-muted">{label} {formatDuration(Number(event.timestamp || 0) / 1000)}</small></span>; }) : <span className="text-[10px] text-cs2-text-muted">无数据</span>}</div></div></div></section>;
    })}</div>
    <aside className="max-h-[460px] overflow-y-auto rounded-xl border border-cs2-border-subtle p-2"><button type="button" onClick={() => setSelectedParticipant("all")} className={`mb-1 w-full rounded-lg px-2 py-1.5 text-left text-[10px] ${selectedParticipant === "all" ? "bg-cyan-400/15 text-cyan-200" : "text-cs2-text-muted hover:bg-white/5"}`}>全部玩家</button>{participants.map((player, index) => <button key={player.puuid || player.participant_id || index} type="button" onClick={() => setSelectedParticipant(String(player.participant_id))} className={`mb-1 flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[10px] ${selectedParticipant === String(player.participant_id) ? "bg-cyan-400/15 text-cyan-200" : "text-cs2-text-muted hover:bg-white/5"}`}><Icon src={getLeagueChampionIconUrl(player.champion_id)} title={player.champion_name} className="h-6 w-6"/><span className="min-w-0 flex-1 truncate">{participantDisplay(player, index, streamerMode, useAliases)}</span></button>)}</aside>
  </div>;
}

function TeamGoldTimeline({ details }) {
  const teamByParticipant = new Map((details?.participants || []).map((player) => [String(player.participant_id), Number(player.team_id)]));
  const teamIds = [...new Set(teamByParticipant.values())].filter(Boolean).slice(0, 4);
  const series = teamIds.map((teamId) => (details?.frames || []).map((frame) => ({
    time: Number(frame.timestamp || 0),
    value: Object.entries(frame.participant_frames || {}).reduce((sum, [participantId, stats]) => teamByParticipant.get(String(participantId)) === teamId ? sum + Number(stats.totalGold || 0) : sum, 0),
  })));
  const maxTime = Math.max(1, ...series.flat().map((point) => point.time));
  const maxValue = Math.max(1, ...series.flat().map((point) => point.value));
  const colors = ["#22d3ee", "#fb7185", "#a78bfa", "#fbbf24"];
  return <div className="rounded-xl border border-cs2-border-subtle p-3"><div className="mb-2 flex flex-wrap gap-3 text-[10px] text-cs2-text-muted">{teamIds.map((teamId, index) => <span key={teamId}><i className="mr-1 inline-block h-2 w-2 rounded-full" style={{ background: colors[index] }}/>队伍 {teamId}</span>)}</div>{series.some((rows) => rows.length > 1) ? <svg viewBox="0 0 800 220" className="h-auto w-full" role="img" aria-label="双方经济时间线"><path d="M35 10V190H790" fill="none" stroke="rgba(255,255,255,.15)"/><g stroke="rgba(255,255,255,.06)">{[1,2,3].map((line) => <path key={line} d={`M35 ${10 + line * 45}H790`}/>)}</g>{series.map((rows, index) => <g key={teamIds[index]}><polyline fill="none" stroke={colors[index]} strokeWidth="3" points={rows.map((point) => `${35 + point.time / maxTime * 755},${190 - point.value / maxValue * 175}`).join(" ")}/>{rows.map((point) => <circle key={point.time} cx={35 + point.time / maxTime * 755} cy={190 - point.value / maxValue * 175} r="5" fill={colors[index]} opacity="0.01"><title>{`${formatDuration(point.time / 1000)} · 队伍 ${teamIds[index]} · ${formatNumber(point.value)} 金币`}</title></circle>)}</g>)}</svg> : <p className="py-8 text-center text-xs text-cs2-text-muted">时间线帧不足，无法绘制经济曲线</p>}</div>;
}

function DifferenceTimeline({ details }) {
  const teamByParticipant = new Map((details?.participants || []).map((player) => [String(player.participant_id), Number(player.team_id)]));
  const teamIds = [...new Set(teamByParticipant.values())].filter(Boolean).slice(0, 2);
  const points = (details?.frames || []).map((frame) => {
    const totals = teamIds.map((teamId) => Object.entries(frame.participant_frames || {}).reduce((sum, [participantId, stats]) => teamByParticipant.get(String(participantId)) === teamId ? sum + Number(stats.totalGold || 0) : sum, 0));
    return { time: Number(frame.timestamp || 0), value: Number(totals[0] || 0) - Number(totals[1] || 0) };
  });
  const maxTime = Math.max(1, ...points.map((point) => point.time));
  const maxAbs = Math.max(1, ...points.map((point) => Math.abs(point.value)));
  return <div className="rounded-xl border border-cs2-border-subtle p-3"><div className="mb-2 text-[10px] text-cs2-text-muted">经济差：队伍 {teamIds[0] || "A"} − 队伍 {teamIds[1] || "B"}</div>{points.length > 1 ? <svg viewBox="0 0 800 220" className="h-auto w-full" role="img" aria-label="双方经济差时间线"><path d="M35 100H790" fill="none" stroke="rgba(255,255,255,.2)"/><path d="M35 10V190" fill="none" stroke="rgba(255,255,255,.12)"/><polyline fill="none" stroke="#22d3ee" strokeWidth="3" points={points.map((point) => `${35 + point.time / maxTime * 755},${100 - point.value / maxAbs * 85}`).join(" ")}/>{points.map((point) => <circle key={point.time} cx={35 + point.time / maxTime * 755} cy={100 - point.value / maxAbs * 85} r="5" fill="#22d3ee" opacity="0.01"><title>{`${formatDuration(point.time / 1000)} · 经济差 ${point.value >= 0 ? "+" : ""}${formatNumber(point.value)}`}</title></circle>)}</svg> : <p className="py-8 text-center text-xs text-cs2-text-muted">时间线帧不足，无法绘制经济差</p>}</div>;
}

const TIMELINE_METRICS = [
  ["totalGold", "总金币"], ["currentGold", "当前金币"], ["level", "等级"], ["xp", "经验"],
  ["cs", "补刀"], ["damageDealt", "造成伤害"], ["damageTaken", "承受伤害"],
];

const CHAMPION_STAT_METRICS = [
  ["生命值", "health", (value, stats) => `${formatNumber(value)} / ${formatNumber(stats.healthMax)}`],
  ["生命回复", "healthRegen"], ["资源值", "power", (value, stats) => `${formatNumber(value)} / ${formatNumber(stats.powerMax)}`],
  ["资源回复", "powerRegen"], ["攻击力", "attackDamage"], ["攻击速度", "attackSpeed", (value) => `${rawValue(value)}%`],
  ["法术强度", "abilityPower"], ["技能急速", "abilityHaste"], ["冷却缩减", "cooldownReduction", (value) => `${rawValue(value)}%`],
  ["护甲", "armor"], ["魔抗", "magicResist"], ["护甲穿透", "armorPen"], ["百分比护穿", "armorPenPercent", (value) => `${rawValue(value)}%`],
  ["额外护穿", "bonusArmorPenPercent", (value) => `${rawValue(value)}%`], ["法术穿透", "magicPen"], ["百分比法穿", "magicPenPercent", (value) => `${rawValue(value)}%`],
  ["额外法穿", "bonusMagicPenPercent", (value) => `${rawValue(value)}%`], ["移动速度", "movementSpeed"], ["生命偷取", "lifesteal", (value) => `${rawValue(value)}%`],
  ["物理吸血", "physicalVamp", (value) => `${rawValue(value)}%`], ["法术吸血", "spellVamp", (value) => `${rawValue(value)}%`],
  ["全能吸血", "omnivamp", (value) => `${rawValue(value)}%`], ["韧性", "ccReduction", (value) => `${rawValue(value)}%`],
];

function participantTimelineValue(stats, metric) {
  if (metric === "cs") return Number(stats.minionsKilled || 0) + Number(stats.jungleMinionsKilled || 0);
  if (metric === "damageDealt") return Number(stats.damageStats?.totalDamageDealt || 0);
  if (metric === "damageTaken") return Number(stats.damageStats?.totalDamageTaken || 0);
  return Number(stats[metric] || 0);
}

function PlayerStatsTimeline({ details, participants, streamerMode, useAliases }) {
  const ids = (details?.participants || []).map((player) => Number(player.participant_id)).filter(Boolean);
  const [participantId, setParticipantId] = useState(ids[0] || 1);
  const [metric, setMetric] = useState("totalGold");
  const [frameIndex, setFrameIndex] = useState(0);
  const rows = (details?.frames || []).map((frame) => ({ time: Number(frame.timestamp || 0), value: participantTimelineValue(frame.participant_frames?.[String(participantId)] || {}, metric) }));
  const selectedFrame = details?.frames?.[Math.min(frameIndex, Math.max(0, (details?.frames?.length || 1) - 1))] || null;
  const selectedFrameStats = selectedFrame?.participant_frames?.[String(participantId)] || {};
  const championStats = selectedFrameStats.championStats || {};
  const visibleChampionStats = CHAMPION_STAT_METRICS.filter(([, key]) => championStats[key] != null);
  const maxTime = Math.max(1, ...rows.map((point) => point.time));
  const maxValue = Math.max(1, ...rows.map((point) => point.value));
  const metricLabel = TIMELINE_METRICS.find(([id]) => id === metric)?.[1] || metric;
  return <div className="rounded-xl border border-cs2-border-subtle p-3"><div className="mb-3 flex flex-wrap gap-2"><select aria-label="时间线玩家" value={participantId} onChange={(event) => setParticipantId(Number(event.target.value))} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs">{ids.map((id, index) => { const player = participants.find((row) => Number(row.participant_id) === id) || details.participants.find((row) => Number(row.participant_id) === id); return <option key={id} value={id}>{participantDisplay(player, index, streamerMode, useAliases)}</option>; })}</select><select value={metric} onChange={(event) => setMetric(event.target.value)} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs">{TIMELINE_METRICS.map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></div>{rows.length > 1 ? <svg viewBox="0 0 800 220" className="h-auto w-full" role="img" aria-label="玩家属性时间线"><path d="M35 10V190H790" fill="none" stroke="rgba(255,255,255,.15)"/><g stroke="rgba(255,255,255,.06)">{[1,2,3].map((line) => <path key={line} d={`M35 ${10 + line * 45}H790`}/>)}</g><polyline fill="none" stroke="#a78bfa" strokeWidth="3" points={rows.map((point) => `${35 + point.time / maxTime * 755},${190 - point.value / maxValue * 175}`).join(" ")}/>{rows.map((point) => <circle key={point.time} cx={35 + point.time / maxTime * 755} cy={190 - point.value / maxValue * 175} r="5" fill="#a78bfa" opacity="0.01"><title>{`${formatDuration(point.time / 1000)} · ${metricLabel} ${formatNumber(point.value)}`}</title></circle>)}</svg> : <p className="py-8 text-center text-xs text-cs2-text-muted">该玩家没有足够的时间线数据</p>}{details?.frames?.length ? <section className="mt-4 border-t border-cs2-border-subtle pt-3"><div className="flex items-center gap-3"><input aria-label="时间线帧" type="range" min="0" max={Math.max(0, details.frames.length - 1)} value={Math.min(frameIndex, details.frames.length - 1)} onChange={(event) => setFrameIndex(Number(event.target.value))} className="min-w-0 flex-1 accent-violet-400"/><b className="w-14 text-right font-mono text-[10px] text-violet-200">{formatDuration(Number(selectedFrame?.timestamp || 0) / 1000)}</b></div>{visibleChampionStats.length ? <div className="mt-3 grid grid-cols-[repeat(auto-fit,minmax(105px,1fr))] gap-2">{visibleChampionStats.map(([label, key, formatter]) => <span key={key} className="rounded-lg bg-white/[.025] p-2 text-[10px] text-cs2-text-muted">{label}<b className="mt-0.5 block text-sm text-cs2-text-primary">{formatter ? formatter(championStats[key], championStats) : rawValue(championStats[key])}</b></span>)}</div> : <p className="mt-3 text-[10px] text-cs2-text-muted">当前数据源没有提供逐帧英雄面板属性；经济、等级、补刀和伤害曲线仍可使用。</p>}<MapPositionPreview mapId={details.map_id} position={selectedFrameStats.position}/></section> : null}</div>;
}

function TimelineTab({ details, participants, streamerMode, useAliases, hideStats = false }) {
  const [section, setSection] = useState("difference");
  return <section className="space-y-3 text-xs">{hideStats ? null : <div className="grid gap-2 sm:grid-cols-3"><Stat icon={MapIcon} label="数据源" value={String(details.source || "LCU").toUpperCase()}/><Stat label="时间线帧" value={formatNumber(details.frame_count)}/><Stat label="事件数量" value={formatNumber(details.event_count)}/></div>}<div className="flex flex-wrap gap-1">{[["difference", "经济差"], ["teams", "队伍经济"], ["player", "玩家属性"]].map(([id, label]) => <button key={id} type="button" onClick={() => setSection(id)} className={`rounded-lg px-3 py-1.5 text-xs font-semibold ${section === id ? "bg-violet-400/15 text-violet-200" : "text-cs2-text-muted hover:bg-white/5"}`}>{label}</button>)}</div>{section === "difference" ? <DifferenceTimeline details={details}/> : null}{section === "teams" ? <TeamGoldTimeline details={details}/> : null}{section === "player" ? <PlayerStatsTimeline details={details} participants={participants} streamerMode={streamerMode} useAliases={useAliases}/> : null}</section>;
}

export default function LeagueDetailedMatchCard({ match, streamerMode = false, useAliases = false, onOpenPlayer, onError, initialDetails = null, onDryRunGame = null }) {
  const [expanded, setExpanded] = useState(false);
  const [tab, setTab] = useState("summary");
  const [matchDetails, setMatchDetails] = useState(initialDetails);
  const [perkMap, setPerkMap] = useState(new Map());
  const [detailsBusy, setDetailsBusy] = useState(false);
  const detailsRequest = useRef(null);
  const perksRequest = useRef(null);
  const disposed = useRef(false);
  const participants = useMemo(() => uniqueParticipants(match), [match]);
  const matchKey = `${match.game_id ?? "unknown"}|${match.source || "auto"}`;
  const matchKeyRef = useRef(matchKey);
  matchKeyRef.current = matchKey;
  const targetPuuid = String(match.participant_puuid ?? match.participantPuuid ?? "");
  const targetRow = participants.find((player) => player.puuid && String(player.puuid) === targetPuuid)
    || participants.find((player) => Number(player.champion_id) === Number(match.champion_id) && String(player.team_id) === String(match.team_id));
  const target = {
    ...match,
    ...(targetRow || {}),
    puuid: targetRow?.puuid || targetPuuid || match.puuid,
    team_id: targetRow?.team_id ?? match.team_id,
    champion_id: targetRow?.champion_id ?? match.champion_id,
    champion_name: targetRow?.champion_name ?? match.champion_name,
    damage: targetRow?.damage ?? match.damage,
    cs: targetRow?.cs ?? match.cs,
    items: normalizedItems(targetRow || match),
    item_slots: itemSlots(targetRow || match),
  };
  const teams = useMemo(() => {
    const groups = new Map();
    for (const player of participants) {
      const teamId = player.team_id ?? "unknown";
      const key = String(teamId);
      if (!groups.has(key)) groups.set(key, { teamId, players: [] });
      groups.get(key).players.push(player);
    }
    return [...groups.values()];
  }, [participants]);
  const ownTeam = participants.filter((player) => String(player.team_id) === String(target.team_id));
  // Tencent's match-history summary can contain only the current participant.
  // A one-row "team" cannot prove either kill participation or team damage share.
  const hasTeamContext = ownTeam.length > 1;
  const teamKills = ownTeam.reduce((sum, player) => sum + Number(player.kills || 0), 0);
  const teamDamage = ownTeam.reduce((sum, player) => sum + Number(player.damage ?? player.totalDamageDealtToChampions ?? 0), 0);
  const kda = (Number(target.kills || 0) + Number(target.assists || 0)) / Math.max(1, Number(target.deaths || 0));
  const kp = hasTeamContext && teamKills ? (Number(target.kills || 0) + Number(target.assists || 0)) / teamKills * 100 : null;
  const damageShare = hasTeamContext && teamDamage ? Number(target.damage ?? target.totalDamageDealtToChampions ?? 0) / teamDamage * 100 : null;
  const matchWin = leagueWinState(target.win ?? match.win);

  useEffect(() => {
    disposed.current = false;
    return () => { disposed.current = true; detailsRequest.current = null; perksRequest.current = null; };
  }, []);
  useEffect(() => {
    setMatchDetails(initialDetails);
    setDetailsBusy(false);
    detailsRequest.current = null;
  }, [matchKey, initialDetails]);

  const ensurePerks = async () => {
    if (perkMap.size) return perkMap;
    if (perksRequest.current) return perksRequest.current;
    const promise = (async () => {
      try {
        const catalog = await fetchLeagueLoadoutCatalog();
        const next = new Map((catalog?.perks || catalog?.styles?.flatMap((style) => style.perks || []) || []).map((perk) => [Number(perk.id), perk]));
        if (!disposed.current) setPerkMap(next);
        return next;
      } catch (error) {
        if (!disposed.current) onError?.(error?.response?.data?.detail || "符文目录读取失败");
        return new Map();
      } finally {
        perksRequest.current = null;
      }
    })();
    perksRequest.current = promise;
    return promise;
  };

  const ensureDetails = async () => {
    if (matchDetails) return matchDetails;
    if (!match.game_id) return null;
    if (detailsRequest.current?.key === matchKey) return detailsRequest.current.promise;
    setDetailsBusy(true);
    const promise = (async () => {
      try {
        const next = await fetchLeagueMatchDetails(match.game_id, match.source || "auto");
        if (!disposed.current && matchKeyRef.current === matchKey) setMatchDetails(next && typeof next === "object" ? next : null);
        return next;
      } catch (error) {
        if (!disposed.current && matchKeyRef.current === matchKey) onError?.(error?.response?.data?.detail || "对局详情读取失败");
        return null;
      } finally {
        if (detailsRequest.current?.key === matchKey) {
          detailsRequest.current = null;
          if (!disposed.current && matchKeyRef.current === matchKey) setDetailsBusy(false);
        }
      }
    })();
    detailsRequest.current = { key: matchKey, promise };
    return promise;
  };

  const selectTab = async (nextTab) => {
    setTab(nextTab);
    if (nextTab === "runes" && !perkMap.size) {
      await ensurePerks();
    }
    if (["events", "builds", "timeline"].includes(nextTab)) await ensureDetails();
  };

  const GoldTimeline = ({ details }) => <TimelineTab details={details} participants={participants} streamerMode={streamerMode} useAliases={useAliases} hideStats/>;

  const championId = assetId(target.champion_id);
  const position = target.position || target.role || "";
  const durationSeconds = Number(match.duration_seconds ?? match.gameDuration ?? 0);
  const cs = Number(target.cs ?? match.cs ?? 0);
  const csPerMinute = durationSeconds > 0 ? (cs / (durationSeconds / 60)).toFixed(1) : "—";
  const targetSpellIds = spellIds(target);
  const itemIds = normalizedItems(target);
  const resultLabel = matchWin === true ? "胜利" : matchWin === false ? "失败" : "未知";
  const winTone = matchWin === true ? "win" : matchWin === false ? "loss" : "neutral";
  const tags = [
    Number(target.penta_kills ?? target.pentaKills) > 0 ? `五杀 ×${target.penta_kills ?? target.pentaKills}` : null,
    Number(target.quadra_kills ?? target.quadraKills) > 0 ? `四杀 ×${target.quadra_kills ?? target.quadraKills}` : null,
    Number(target.triple_kills ?? target.tripleKills) > 0 ? `三杀 ×${target.triple_kills ?? target.tripleKills}` : null,
    Number(target.double_kills ?? target.doubleKills) > 0 ? `双杀 ×${target.double_kills ?? target.doubleKills}` : null,
  ].filter(Boolean);

  return <article data-testid="league-match-card" className={`relative w-full min-w-0 overflow-hidden ${expanded ? "" : ""}`}>
    <div className="relative box-border flex min-h-[128px] w-full overflow-hidden rounded border border-solid border-white/10 bg-neutral-900/95 select-none dark:bg-neutral-900/95">
      <div className="z-10 flex min-w-0 flex-1 gap-2 px-4 py-1">
        <div className="z-20 my-1 flex min-w-0 flex-1 flex-col justify-between">
          <div className="flex h-12 gap-2">
            <div className="flex w-[70px] shrink-0 items-center">
              <div className="relative">
                <Icon src={championId ? getLeagueChampionIconUrl(championId) : ""} title={target.champion_name} className={`relative -left-0.5 h-11 w-11 rounded-lg border-2 ${matchWin === true ? "border-blue-300/80" : matchWin === false ? "border-red-300/80" : "border-white/70"}`} />
                {position ? <span className="absolute bottom-0 right-0 rounded-sm bg-black/80 px-1 text-[9px] text-white/80">{position}</span> : null}
                {Number(target.kills || 0) >= 10 ? <Crown className="absolute -top-2 left-1/2 h-3.5 w-3.5 -translate-x-1/2 text-yellow-400" /> : null}
              </div>
            </div>

            <div className="flex min-w-0 flex-1 items-center gap-2">
              <div className="flex w-[84px] shrink-0 gap-0.5">
                {(targetSpellIds.length || target.perks?.length) ? <><SpellIcons player={target} compact/><div className="flex flex-col gap-0.5">{target.perks?.[0] ? <Icon src={getLeaguePerkIconUrl(assetId(target.perks[0]))} title="主系符文" className="h-6 w-6"/> : null}{target.perks?.[1] ? <Icon src={getLeaguePerkIconUrl(assetId(target.perks[1]))} title="副系符文" className="h-6 w-6"/> : null}</div></> : null}
              </div>
              <div className={`${MATCH_METRIC_GRID} items-center`}>
                <div className="min-w-0 text-center">
                  <div className="flex items-center justify-center gap-0.5"><b className="text-base text-white">{target.kills ?? 0}</b><span className="mx-px text-xs text-white/60">/</span><b className="text-base text-red-300">{target.deaths ?? 0}</b><span className="mx-px text-xs text-white/60">/</span><b className="text-base text-white">{target.assists ?? 0}</b></div>
                  <div className="flex justify-center gap-1 text-xs text-white/80">{Number(target.deaths || 0) === 0 && (Number(target.kills || 0) > 0 || Number(target.assists || 0) > 0) ? <span className="text-yellow-200">完美</span> : <span>{kda.toFixed(2)}</span>}<span>({kp == null ? "—" : `${kp.toFixed(0)}%`})</span><span className="sr-only">参团</span></div>
                </div>
                <div className="min-w-0 text-center"><b className="block text-base text-white">{damageShare == null ? "—" : `${damageShare.toFixed(0)}%`}</b><span className="text-xs text-white/70">{formatNumber(target.damage ?? target.totalDamageDealtToChampions)} 伤害</span><span className="sr-only">伤害占比</span></div>
                <div className="hidden min-w-0 text-center min-[700px]:block"><b className="block text-base text-white">{formatNumber(cs)} <small className="text-[11px] font-normal text-white/60">补兵</small></b><span className="text-xs text-white/70">{csPerMinute} / 分钟</span></div>
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <div className={`w-[70px] shrink-0 text-sm font-bold leading-none ${matchWin === true ? "text-blue-300" : matchWin === false ? "text-red-300" : "text-white"}`}>{resultLabel}</div>
            <div className="flex gap-0.5">{itemIds.slice(0, 6).map((id, index) => <Icon key={`${id}-${index}`} src={getLeagueItemIconUrl(id)} title={`装备 ${id}`} className="h-5 w-5"/>)}{itemIds[6] ? <Icon src={getLeagueItemIconUrl(itemIds[6])} title={`饰品 ${itemIds[6]}`} className="h-5 w-5 rounded-full"/> : null}</div>
            <div className="min-w-0 flex-1 truncate">{tags.map((tag) => <span key={tag} className="mr-1 inline-flex rounded bg-white/10 px-1.5 py-0.5 text-[9px] text-white/70">{tag}</span>)}</div>
          </div>

          <div className="flex min-w-0 items-center text-xs text-white/70"><span className="text-white/85">{queueName(match)}</span><span className="mx-1 text-white/40">·</span><span>{formatDuration(durationSeconds)}</span><span className="mx-1 text-white/40">·</span><span title={formatLeagueTimestamp(match.played_at)}>{relativeTime(match.played_at)}</span><span className="mx-1 text-white/40">·</span><span className="min-w-0 truncate">{mapName(match)}</span>{kp == null || damageShare == null ? <span className="sr-only">—</span> : null}</div>
        </div>

        {participants.length ? <div className="z-20 my-1 hidden w-[168px] max-w-[168px] gap-2 lg:flex">{teams.slice(0, 2).map((team) => <div key={String(team.teamId)} className="flex min-w-0 flex-1 flex-col justify-between gap-0.5">{team.players.slice(0, 5).map((player, index) => <button key={player.puuid || player.participant_id || index} type="button" disabled={!player.puuid} onClick={() => player.puuid && onOpenPlayer?.(player.puuid)} className="group flex min-w-0 cursor-pointer items-center gap-1 text-left"><Icon src={assetId(player.champion_id) ? getLeagueChampionIconUrl(assetId(player.champion_id)) : ""} title={player.champion_name} className={`h-4 w-4 shrink-0 rounded ${String(player.puuid) === String(target.puuid) ? "ring-1 ring-white/70" : ""}`}/><span className={`min-w-0 truncate text-xs transition-colors group-hover:text-white ${String(player.puuid) === String(target.puuid) ? "font-bold text-white/90" : "text-white/70"}`}>{playerName(player, index, streamerMode, useAliases)}</span></button>)}</div>)}</div> : null}
      </div>

      <button type="button" aria-label={expanded ? "收起战绩详情" : "展开战绩详情"} onClick={() => setExpanded((value) => !value)} className="z-20 flex w-8 shrink-0 cursor-pointer items-center justify-center border-l border-white/10 bg-white/5 text-base text-white/60 transition-colors hover:bg-white/10 hover:text-white"><ChevronDown className={`h-4 w-4 transition-transform ${expanded ? "rotate-180" : "-rotate-90"}`} /></button>
      <div className={`pointer-events-none absolute inset-0 z-0 ${winTone === "win" ? "shadow-[inset_3px_0_0_rgba(147,197,253,.95),inset_0_0_26px_rgba(59,130,246,.08)]" : winTone === "loss" ? "shadow-[inset_3px_0_0_rgba(252,165,165,.95),inset_0_0_26px_rgba(239,68,68,.08)]" : "shadow-[inset_3px_0_0_rgba(255,255,255,.45)]"}`} />
    </div>
    {expanded ? <div className="border-t border-cs2-border-subtle p-3">
      <div className="mb-3 flex flex-wrap items-center gap-1"><div className="flex min-w-0 flex-1 flex-wrap gap-1">{TABS.map(([id, label]) => <button key={id} type="button" onClick={() => void selectTab(id)} className={`rounded-lg px-3 py-1.5 text-xs font-semibold ${tab === id ? "bg-cyan-400/15 text-cyan-200" : "text-cs2-text-muted hover:bg-white/5"}`}>{label}</button>)}</div><div className="flex gap-1"><LeagueMatchReplayActions match={match} onError={onError}/>{onDryRunGame ? <button type="button" aria-label="载入实时面板模拟" title="将历史阵容载入实时对局面板（只读）" onClick={() => onDryRunGame(buildLeagueHistoricalPreview(match, participants))} className="rounded-lg border border-cyan-400/30 bg-cyan-400/10 px-2.5 py-1.5 text-xs font-semibold text-cyan-200">模拟</button> : null}</div></div>
      {tab === "summary" ? <div className="space-y-3">{teams.map((team) => { const teamWin = leagueWinState(team.players[0]?.win); return <section key={team.teamId}><h4 className="mb-1.5 text-[11px] font-bold text-cs2-text-muted">队伍 {team.teamId} · {teamWin === true ? "胜利" : teamWin === false ? "失败" : "结果未知"}</h4><TeamTable players={team.players} targetPuuid={targetPuuid} streamerMode={streamerMode} useAliases={useAliases} onOpenPlayer={onOpenPlayer}/></section>; })}</div> : null}
      {tab === "details" ? <RawDetailsTab match={match} participants={participants} streamerMode={streamerMode} useAliases={useAliases}/> : null}
      {tab === "runes" ? <RunesTab participants={participants} perkMap={perkMap} streamerMode={streamerMode} useAliases={useAliases}/> : null}
      {tab === "events" ? detailsBusy ? <p className="rounded-xl border border-cs2-border-subtle p-8 text-center text-xs text-cs2-text-muted">正在读取本局事件…</p> : matchDetails ? <EventsTab details={matchDetails} participants={participants} mapId={match.map_id} streamerMode={streamerMode} useAliases={useAliases}/> : <p className="text-xs text-cs2-text-muted">此对局没有可用事件数据。</p> : null}
      {tab === "builds" ? detailsBusy ? <p className="rounded-xl border border-cs2-border-subtle p-8 text-center text-xs text-cs2-text-muted">正在读取出装过程…</p> : matchDetails ? <BuildsTab details={matchDetails} participants={participants} streamerMode={streamerMode} useAliases={useAliases}/> : <p className="text-xs text-cs2-text-muted">此对局没有可用出装过程。</p> : null}
      {tab === "timeline" ? <section className="space-y-3 text-xs">{detailsBusy ? <p className="rounded-xl border border-cs2-border-subtle p-8 text-center text-cs2-text-muted">正在读取本局时间线…</p> : matchDetails ? <><div className="grid gap-2 sm:grid-cols-3"><Stat icon={MapIcon} label="数据源" value={String(matchDetails.source || match.source || "LCU").toUpperCase()}/><Stat label="时间线帧" value={formatNumber(matchDetails.frame_count)}/><Stat label="事件数量" value={formatNumber(matchDetails.event_count)}/></div><GoldTimeline details={matchDetails}/></> : <p className="text-cs2-text-muted">此对局暂时没有可用时间线。</p>}</section> : null}
    </div> : null}
  </article>;
}
