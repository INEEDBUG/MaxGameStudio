import { useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp, RefreshCw, Users } from "lucide-react";
import { fetchLeagueLabStatus, fetchLeagueOngoingGame } from "../../api/leagueLabApi";
import { getLeagueChampionIconUrl, getLeagueProfileIconUrl } from "../../api/api";
import { maskLeagueName } from "../../utils/leagueStreamerMode";
import { leagueWinState, normalizeLeagueTimestamp } from "../../utils/leagueDisplay";
import LeagueDetailedMatchCard from "./LeagueDetailedMatchCard";

const TAG_TONES = {
  positive: "bg-emerald-400/10 text-emerald-200",
  negative: "bg-rose-400/10 text-rose-200",
  warning: "bg-amber-400/10 text-amber-200",
  info: "bg-cyan-400/10 text-cyan-200",
};

const QUEUE_LABELS = {
  RANKED_SOLO_5x5: "单双排",
  RANKED_FLEX_SR: "灵活组排",
  RANKED_TFT: "云顶之弈",
  RANKED_TFT_DOUBLE_UP: "双人作战",
};

const HISTORY_QUEUE_LABELS = {
  400: "普通对战",
  420: "单双排位",
  430: "匹配对战",
  440: "灵活排位",
  450: "极地大乱斗",
  490: "快速游戏",
};

function objectValue(value) {
  return value && typeof value === "object" ? value : {};
}

function boolValue(value) {
  return value === true || value === 1 || value === "true";
}

function numberValue(value, fallback = 0) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function formatMetric(value, digits = 0) {
  if (value == null || value === "") return "—";
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return String(value);
  return digits > 0 ? parsed.toFixed(digits) : parsed.toLocaleString("zh-CN");
}

function formatWinRate(wins, matches) {
  const total = numberValue(matches);
  return total > 0 ? `${Math.round(numberValue(wins) / total * 100)}%` : "—";
}

function historyRows(player) {
  const candidates = [
    player?.recent_matches,
    player?.match_history,
    player?.matches,
    player?.games?.games,
    player?.games,
  ];
  return candidates.find((value) => Array.isArray(value)) || [];
}

function unwrapHistoryGame(value) {
  const row = objectValue(value);
  if (row.json && typeof row.json === "object") return row.json;
  if (row.game && typeof row.game === "object") return row.game;
  return row;
}

function statValue(row, key, fallback = 0) {
  const stats = objectValue(row?.stats);
  return stats[key] ?? row?.[key] ?? fallback;
}

function normalizeHistoryMatches(player) {
  const puuid = String(player?.puuid || "");
  return historyRows(player).map((entry, index) => {
    const game = unwrapHistoryGame(entry);
    if (game.game_id != null || game.gameId == null && !Array.isArray(game.participants)) {
      if (game.game_id == null) return null;
      return { ...game, participant_puuid: game.participant_puuid || puuid, _history_index: index };
    }
    const rows = Array.isArray(game.participants) ? game.participants : [];
    if (!rows.length) return null;
    const identities = Array.isArray(game.participantIdentities) ? game.participantIdentities : [];
    const identityById = new Map(identities.map((identity) => [String(identity?.participantId), objectValue(identity)]));
    const targetIdentity = identities.find((identity) => String(identity?.player?.puuid || identity?.puuid || "") === puuid);
    const targetId = targetIdentity?.participantId;
    const target = rows.find((row) => String(row?.puuid || "") === puuid || targetId != null && String(row?.participantId) === String(targetId));
    if (!target) return null;
    const normalizeParticipant = (row) => {
      const identity = identityById.get(String(row?.participantId)) || {};
      const identityPlayer = objectValue(identity.player);
      const championId = numberValue(row?.championId);
      return {
        participant_id: row?.participantId,
        puuid: row?.puuid || identityPlayer.puuid || identity.puuid,
        game_name: row?.riotIdGameName || row?.gameName || row?.summonerName || identityPlayer.gameName || identityPlayer.displayName || "",
        tag_line: row?.riotIdTagline || row?.tagLine || identityPlayer.tagLine || "",
        profile_icon_id: row?.profileIcon || row?.profileIconId || identityPlayer.profileIcon,
        team_id: row?.teamId,
        champion_id: championId,
        champion_name: row?.championName || row?.champion_name || (championId ? `英雄 ${championId}` : "未知英雄"),
        position: row?.teamPosition || row?.timeline?.lane || "",
        role: row?.individualPosition || row?.timeline?.role || "",
        spell1_id: row?.spell1Id ?? row?.summoner1Id,
        spell2_id: row?.spell2Id ?? row?.summoner2Id,
        kills: statValue(row, "kills"),
        deaths: statValue(row, "deaths"),
        assists: statValue(row, "assists"),
        win: boolValue(statValue(row, "win", row?.win)),
        gold: statValue(row, "goldEarned", row?.gold),
        level: statValue(row, "champLevel", row?.level),
        gold_spent: statValue(row, "goldSpent", row?.gold_spent),
        cs: numberValue(statValue(row, "totalMinionsKilled")) + numberValue(statValue(row, "neutralMinionsKilled")),
        damage: statValue(row, "totalDamageDealtToChampions", row?.damage),
        damage_taken: statValue(row, "totalDamageTaken", row?.damage_taken),
        healing: statValue(row, "totalHeal", row?.healing),
        time_ccing: statValue(row, "totalTimeCCDealt", row?.time_ccing),
        tower_damage: statValue(row, "damageDealtToTurrets", row?.tower_damage),
        vision_score: statValue(row, "visionScore", row?.vision_score),
        items: row?.items || Array.from({ length: 7 }, (_, itemIndex) => statValue(row, `item${itemIndex}`, null)).filter((value) => value != null && numberValue(value) > 0),
        perks: row?.perks || Array.from({ length: 6 }, (_, perkIndex) => statValue(row, `perk${perkIndex}`, null)).filter((value) => value != null && numberValue(value) > 0),
        augments: row?.augments || Array.from({ length: 6 }, (_, augmentIndex) => statValue(row, `playerAugment${augmentIndex + 1}`, null)).filter((value) => value != null && numberValue(value) > 0),
        challenges: row?.challenges || objectValue(row?.stats).challenges || {},
        raw_stats: row?.raw_stats || {},
      };
    };
    const participants = rows.map(normalizeParticipant);
    const targetParticipant = participants.find((row) => String(row.puuid || "") === puuid) || normalizeParticipant(target);
    return {
      game_id: game.gameId ?? game.game_id,
      played_at: game.gameCreationDate ?? game.gameCreation ?? game.gameStartTimestamp ?? game.played_at,
      duration_seconds: game.gameDuration ?? game.duration_seconds,
      game_mode: game.gameMode ?? game.game_mode,
      game_type: game.gameType ?? game.game_type,
      game_version: game.gameVersion ?? game.game_version,
      map_id: game.mapId ?? game.map_id,
      queue_id: game.queueId ?? game.queue_id,
      participant_puuid: puuid || targetParticipant.puuid,
      team_id: targetParticipant.team_id,
      participants,
      ...targetParticipant,
      _history_index: index,
    };
  }).filter(Boolean);
}

function rankedRows(ranked) {
  const value = objectValue(ranked);
  let rows = [];
  if (Array.isArray(value.queues)) rows = value.queues;
  else if (value.queueMap && typeof value.queueMap === "object") rows = Object.entries(value.queueMap).map(([queueType, row]) => ({ ...objectValue(row), queueType }));
  else if (Array.isArray(value.rankedStats)) rows = value.rankedStats;
  else if (value.tier || value.rank || value.division || value.leaguePoints != null) rows = [value];
  return rows.map((row) => {
    const queueType = row.queueType || row.queue || row.queueId || row.queue_type;
    const tier = row.tier || row.rankTier || row.division;
    const rank = row.rank || row.divisionRank || row.subTier;
    const wins = row.wins ?? row.win;
    const losses = row.losses ?? row.lose;
    const leaguePoints = row.leaguePoints ?? row.lp;
    if (!queueType && !tier && wins == null && losses == null && leaguePoints == null) return null;
    return { queueType, tier, rank, wins, losses, leaguePoints };
  }).filter(Boolean).slice(0, 4);
}

function playerAnalysis(player, data) {
  return player?.analysis || data?.analysis?.players?.[player?.puuid] || {};
}

function playerSummonerName(player, index) {
  return String(player?.summoner?.gameName || player?.summoner?.game_name || player?.summoner?.displayName || player?.game_name || player?.champion_name || `玩家 ${index + 1}`);
}

function playerTagLine(player) {
  return String(player?.summoner?.tagLine || player?.summoner?.tag_line || "");
}

function playerProfileIconId(player) {
  return player?.summoner?.profileIconId ?? player?.summoner?.profile_icon_id ?? player?.summoner?.profileIcon;
}

function analysisValue(analysis, keys, fallback = null) {
  for (const key of keys) {
    const value = key.split(".").reduce((current, part) => current?.[part], analysis);
    if (value != null && value !== "") return value;
  }
  return fallback;
}

function historyQueueLabel(game) {
  const queueId = Number(game?.queue_id ?? game?.queueId);
  return HISTORY_QUEUE_LABELS[queueId] || game?.queue_name || game?.game_mode || game?.gameMode || game?.game_type || "对局";
}

function historyTimeLabel(value) {
  const timestamp = normalizeLeagueTimestamp(value);
  if (timestamp == null) return "时间未知";
  const date = new Date(timestamp);
  const pad = (part) => String(part).padStart(2, "0");
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function historyResult(game) {
  const direct = leagueWinState(game?.win ?? game?.win_result ?? game?.winResult);
  if (direct != null) return direct;
  const result = String(game?.win_result ?? game?.winResult ?? game?.win ?? "").toLowerCase();
  if (["win", "won"].includes(result)) return true;
  if (["loss", "lose", "lost", "failed"].includes(result)) return false;
  return null;
}

function miniHistoryRows(player) {
  return normalizeHistoryMatches(player).slice(0, 8).map((game, index) => {
    const result = historyResult(game);
    return {
      id: game?.game_id ?? game?.gameId ?? `history-${index}`,
      gameId: game?.game_id ?? game?.gameId,
      championId: Number(game?.champion_id ?? game?.championId ?? 0),
      championName: game?.champion_name ?? game?.championName ?? "未知英雄",
      kills: Number(game?.kills ?? game?.stats?.kills ?? 0),
      deaths: Number(game?.deaths ?? game?.stats?.deaths ?? 0),
      assists: Number(game?.assists ?? game?.stats?.assists ?? 0),
      win: result,
      playedAt: game?.played_at ?? game?.gameCreation ?? game?.gameCreationDate,
      playedAtLabel: historyTimeLabel(game?.played_at ?? game?.gameCreation ?? game?.gameCreationDate),
      queue: historyQueueLabel(game),
      resultLabel: result === true ? "胜利" : result === false ? "失败" : "未知",
    };
  });
}

function idleCopy(data) {
  if (data?.settings?.enabled === false) return "实时对局功能已关闭";
  if (data?.is_connected === false || data?.connected === false) return "尚未连接英雄联盟客户端";
  if (data?.is_spectating || data?.spectating) return "正在等待观战对局";
  if (data?.phase === "PreEndOfGame" || data?.phase === "EndOfGame" || data?.phase === "WaitingForStats") return "正在等待本局结算完成";
  return "当前没有进行中的对局";
}

function Metric({ label, value, title }) {
  return <span title={title} className="rounded-lg bg-black/10 px-2 py-1.5 text-[10px] text-cs2-text-muted"><span className="block">{label}</span><b className="mt-0.5 block text-sm text-cs2-text-primary">{value}</b></span>;
}

function PlayerDetails({ player, privacy, recentMatches, onOpenPlayer, onError }) {
  const recent = player.recent || {};
  const usage = player.champion_usage || {};
  const ranks = rankedRows(player.ranked);
  const tags = Array.isArray(player.performance_tags) ? player.performance_tags : [];
  const unavailable = Array.isArray(player.data_availability?.unavailable) ? player.data_availability.unavailable : [];
  const unavailableLabels = unavailable.map((key) => ({ summoner: "召唤师资料", ranked: "排位信息", history: "近期战绩", mastery: "英雄熟练度" }[key] || key));
  return <div data-testid="player-details" className="border-t border-cs2-border-subtle px-3 pb-3 pt-3">
    {unavailableLabels.length ? <p data-testid="player-data-unavailable" className="mb-3 rounded-xl border border-amber-400/25 bg-amber-400/10 px-3 py-2 text-[11px] text-amber-100">客户端未向当前账号开放：{unavailableLabels.join("、")}。其余已成功读取的数据仍会正常展示。</p> : null}
    <div className="grid gap-2 sm:grid-cols-3 lg:grid-cols-6">
      <Metric label="近期样本" value={`${formatMetric(recent.matches)} 场`} title="客户端返回的近期战绩样本数。"/>
      <Metric label="近期胜率" value={formatWinRate(recent.wins, recent.matches)} title="近期样本中的胜场占比。"/>
      <Metric label="平均 KDA" value={formatMetric(recent.average_kda, 2)} title="近期样本的平均 (击杀+助攻)/死亡。"/>
      <Metric label="Akari Score" value={formatMetric(recent.akari_score, 2)} title="后端根据已分析样本计算的 Akari 聚合分。"/>
      <Metric label="已分析详情" value={`${formatMetric(recent.details_analyzed)} 场`} title="参与标签和评分计算的近期对局数量。"/>
      <Metric label="当前分路" value={player.position || "—"} title="客户端当前返回的分路。"/>
    </div>
    <div className="mt-3 grid gap-3 lg:grid-cols-2">
      <section className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3">
        <h4 className="text-[11px] font-bold text-cs2-text-secondary">当前英雄使用</h4>
        {usage.mode === "mastery" ? <div className="mt-2 grid grid-cols-2 gap-2 text-xs"><Metric label="熟练度等级" value={formatMetric(usage.mastery_level)} title="客户端返回的英雄熟练度等级。"/><Metric label="熟练度点数" value={formatMetric(usage.mastery_points)} title="客户端返回的英雄熟练度点数。"/></div> : usage.mode === "recent" ? <div className="mt-2 grid grid-cols-3 gap-2 text-xs"><Metric label="使用场次" value={formatMetric(usage.matches)} title="近期样本中使用当前英雄的场次。"/><Metric label="胜率" value={formatWinRate(usage.wins, usage.matches)} title="近期样本中使用当前英雄的胜率。"/><Metric label="平均 KDA" value={formatMetric(usage.average_kda, 2)} title="近期样本中使用当前英雄的平均 KDA。"/></div> : <p className="mt-2 text-[11px] text-cs2-text-muted">当前设置未返回英雄使用样本。</p>}
      </section>
      <section className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3">
        <h4 className="text-[11px] font-bold text-cs2-text-secondary">排位信息</h4>
        {ranks.length ? <div className="mt-2 space-y-1.5">{ranks.map((rank, index) => <div key={`${rank.queueType || "rank"}-${index}`} className="flex items-center justify-between gap-2 text-[11px]"><span className="text-cs2-text-muted">{QUEUE_LABELS[rank.queueType] || rank.queueType || "排位"}</span><span className="text-right"><b>{[rank.tier, rank.rank].filter(Boolean).join(" ") || "未定级"}</b>{rank.leaguePoints != null ? <span className="ml-2 text-cyan-200">{formatMetric(rank.leaguePoints)} LP</span> : null}{rank.wins != null || rank.losses != null ? <small className="ml-2 text-cs2-text-muted">{formatMetric(rank.wins)}胜 / {formatMetric(rank.losses)}负</small> : null}</span></div>)}</div> : <p className="mt-2 text-[11px] text-cs2-text-muted">{unavailable.includes("ranked") ? "当前客户端未开放该玩家的排位资料。" : "当前 payload 没有排位明细。"}</p>}
      </section>
    </div>
    <section className="mt-3 rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3">
      <h4 className="text-[11px] font-bold text-cs2-text-secondary">标签解释</h4>
      {tags.length ? <div data-testid="player-tag-explanations" className="mt-2 space-y-1.5">{tags.map((tag) => <div key={tag.id || tag.label} title={tag.title || tag.label} className="flex items-start gap-2 text-[11px]"><span className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] font-semibold ${TAG_TONES[tag.tone] || TAG_TONES.info}`}>{tag.label}</span><span className="text-cs2-text-muted">{tag.title || "客户端没有返回标签解释。"}</span></div>)}</div> : <p className="mt-2 text-[11px] text-cs2-text-muted">当前没有可解释的标签。</p>}
    </section>
    <section className="mt-3">
      <div className="mb-2 flex items-center justify-between gap-2"><h4 className="text-[11px] font-bold text-cs2-text-secondary">近期对局</h4><span className="text-[10px] text-cs2-text-muted">展开单局卡片后，事件/出装/时间线才会按需读取详情</span></div>
      {recentMatches.length ? <div data-testid="player-recent-matches" className="space-y-2">{recentMatches.map((match, index) => <LeagueDetailedMatchCard key={match.game_id || match._history_index || index} match={match} streamerMode={privacy.enabled} useAliases={privacy.aliases} onOpenPlayer={onOpenPlayer} onError={onError}/>)}</div> : <p className="rounded-xl border border-dashed border-cs2-border-subtle p-5 text-center text-[11px] text-cs2-text-muted">{unavailable.includes("history") ? "当前客户端未开放该玩家的近期战绩。" : "暂无可展示的近期对局；当前卡片只显示客户端已返回的聚合指标。"}</p>}
    </section>
  </div>;
}

const TEAM_META = {
  "100": { label: "蓝队", dot: "bg-sky-400", text: "text-sky-300", border: "border-sky-400/30" },
  "200": { label: "红队", dot: "bg-rose-400", text: "text-rose-300", border: "border-rose-400/30" },
  "TEAM-100": { label: "蓝队", dot: "bg-sky-400", text: "text-sky-300", border: "border-sky-400/30" },
  "TEAM-200": { label: "红队", dot: "bg-rose-400", text: "text-rose-300", border: "border-rose-400/30" },
  LOBBY: { label: "当前房间", dot: "bg-cyan-400", text: "text-cyan-300", border: "border-cyan-400/30" },
};

const MODE_LABELS = {
  CLASSIC: "经典模式",
  ARAM: "海克斯大乱斗",
  URF: "无限火力",
  CHERRY: "斗魂竞技场",
  TFT: "云顶之弈",
};

const MAP_LABELS = {
  11: "召唤师峡谷",
  12: "水晶之痕",
  21: "极地大乱斗",
  30: "斗魂竞技场",
};

function ongoingTeamMeta(team) {
  return TEAM_META[String(team)] || { label: String(team || "未知队伍"), dot: "bg-white/50", text: "text-white/75", border: "border-white/15" };
}

function queueMeta(data) {
  const queue = objectValue(data?.queue);
  const mode = String(queue.gameMode || queue.game_mode || data?.game_mode || "").toUpperCase();
  const mapId = Number(queue.mapId || queue.map_id || data?.map_id || 0);
  const modeLabel = queue.gameModeName || queue.modeName || MODE_LABELS[mode] || mode || "英雄联盟";
  const mapLabel = queue.mapName || queue.map_name || MAP_LABELS[mapId] || (mode === "ARAM" ? "极地大乱斗" : "随机地图");
  const queueId = queue.id ?? queue.queueId ?? queue.queue_id;
  return { modeLabel, mapLabel, queueId, title: `${modeLabel} · ${mapLabel}` };
}

function teamSummary(team, players, data) {
  const external = data?.analysis?.teams?.[String(team)] || data?.analysis?.teams?.[team];
  if (external) return external;
  const rows = players.map((player) => player?.recent || {}).filter((row) => Number(row.matches || 0) > 0);
  const games = rows.reduce((sum, row) => sum + Number(row.matches || 0), 0);
  const wins = rows.reduce((sum, row) => sum + Number(row.wins || 0), 0);
  const kdaValues = rows.map((row) => Number(row.average_kda || 0));
  return {
    games,
    wins,
    avgWinRate: games ? wins / games : null,
    avgKda: kdaValues.length ? kdaValues.reduce((sum, value) => sum + value, 0) / kdaValues.length : null,
    players: players.map((player) => player.puuid).filter(Boolean),
  };
}

function formatTeamStat(value, digits = 0) {
  if (value == null || !Number.isFinite(Number(value))) return "—";
  return digits ? Number(value).toFixed(digits) : Number(value).toFixed(0);
}

function playerRankLabel(player) {
  const first = rankedRows(player?.ranked)[0];
  if (!first) return "未分配";
  return [first.tier, first.rank].filter(Boolean).join(" ") || "未定级";
}

function playerLevel(player) {
  return player?.summoner?.summonerLevel ?? player?.summoner?.level ?? player?.summoner_level;
}

function PlayerHistoryRow({ item, onPreviewGame, puuid }) {
  const rowTone = item.win === true ? "bg-blue-500/25 text-blue-100" : item.win === false ? "bg-rose-500/25 text-rose-100" : "bg-white/[.07] text-white/70";
  const resultTone = item.win === true ? "text-blue-200" : item.win === false ? "text-rose-200" : "text-white/55";
  return <button key={item.id} type="button" onClick={() => item.gameId && onPreviewGame?.({ summary: { ...item, game_id: item.gameId, participant_puuid: puuid }, puuid })} title="打开该场对局详情" className={`group flex h-9 w-full items-center gap-1.5 rounded px-1.5 py-0.5 text-left text-[10px] transition-[filter] duration-150 hover:brightness-110 ${rowTone}`}>
    <span className="h-6 w-6 shrink-0 overflow-hidden rounded bg-white/10 text-center leading-6">{item.championId ? <img src={getLeagueChampionIconUrl(item.championId)} alt={item.championName} className="h-6 w-6 rounded object-cover"/> : "?"}</span>
    <span className="min-w-0 flex-1 leading-4"><span className="block truncate">{item.queue}</span><span className="block truncate text-[9px] text-white/55"><span>{item.playedAtLabel}</span><span className={`ml-1 ${resultTone}`}>{item.resultLabel}</span></span></span>
    <span className="shrink-0 whitespace-nowrap tabular-nums text-[10px]">{item.kills} / {item.deaths} / {item.assists}</span>
  </button>;
}

function PlayerCard({ player, index, data, privacy, onOpenPlayer, onError, onPreviewGame }) {
  const [expanded, setExpanded] = useState(false);
  const recent = player.recent || {};
  const usage = player.champion_usage || {};
  const jungle = player.jungle_analysis || {};
  const match = player.match_stats || {};
  const recentMatches = normalizeHistoryMatches(player);
  const analysis = playerAnalysis(player, data);
  const rawName = playerSummonerName(player, index);
  const playerName = privacy.enabled ? maskLeagueName(rawName, index, privacy.aliases, player.puuid) : rawName;
  const tagLine = playerTagLine(player);
  const iconId = playerProfileIconId(player);
  const cardBorder = data?.show_match_history_item_border ? "border-cyan-400/30" : "border-white/10";
  const winRate = analysisValue(analysis, ["summary.winRate", "win_rate"], recent.matches ? Number(recent.wins || 0) / Number(recent.matches) : null);
  const averageKda = analysisValue(analysis, ["summary.avgKda", "summary.averageKda", "average_kda"], recent.average_kda);
  const akariScore = analysisValue(analysis, ["akariScore.total", "akari_score"], recent.akari_score);
  const position = String(player.position || player.selectedPosition || player.assignedPosition || "").toUpperCase();
  const historyPreview = miniHistoryRows(player);
  const usageLine = usage.mode === "mastery"
    ? `${player.champion_name || "当前英雄"} · 熟练度 ${usage.mastery_level || 0} / ${numberValue(usage.mastery_points).toLocaleString()} 点`
    : usage.mode === "none"
      ? `近 ${recent.matches || 0} 场胜率 ${recent.matches ? Math.round(recent.wins / recent.matches * 100) : 0}% · KDA ${recent.average_kda || 0}`
      : `近 ${recent.matches || 0} 场胜率 ${recent.matches ? Math.round(recent.wins / recent.matches * 100) : 0}% · ${player.champion_name || "当前英雄"} ${usage.matches || 0} 场 / KDA ${usage.average_kda || 0}`;
  const analysisTags = Array.isArray(player.performance_tags) ? player.performance_tags : [];
  const championUsage = Array.isArray(analysis?.champions) ? analysis.champions : Object.values(analysis?.champions || {});
  const championUsageRows = championUsage.length ? championUsage : (player.rating_summary?.main_champions || []);
  const outlier = data?.analysis?.players?.[player.puuid]?.kda_outlier || player?.analysis?.kda_outlier;
  return <article data-testid={`ongoing-player-card-${player.puuid || index}`} className={`relative flex h-[375px] min-h-[375px] min-w-0 flex-col overflow-hidden rounded-lg border bg-[#17181c]/95 ${cardBorder}`}>
    {player.premade_group ? <span aria-label={`组排 ${player.premade_group}`} className="absolute right-0 top-0 h-4 w-4 translate-x-1/2 -translate-y-1/2 rotate-45 bg-violet-400/80"/> : null}
    <div className="flex items-start gap-2 border-b border-white/10 p-2">
      <button type="button" disabled={!player.puuid} aria-label={`${playerName} 头像`} title="打开玩家中心" onClick={() => player.puuid && onOpenPlayer(player.puuid)} className="relative h-10 w-10 shrink-0 rounded-full bg-white/[.05] disabled:cursor-default">
        {player.champion_id ? <img src={getLeagueChampionIconUrl(player.champion_id)} alt={player.champion_name || "英雄"} className="h-10 w-10 rounded-full object-cover"/> : iconId != null ? <img src={getLeagueProfileIconUrl(iconId)} alt="召唤师头像" className="h-10 w-10 rounded-full object-cover"/> : <span className="grid h-10 w-10 place-items-center text-xs text-cs2-text-muted">?</span>}
        {iconId != null && player.champion_id ? <img src={getLeagueProfileIconUrl(iconId)} alt="召唤师头像" className="absolute -bottom-1 -right-1 h-4 w-4 rounded-full border border-cs2-bg-elevated object-cover"/> : null}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1">
          <button type="button" disabled={!player.puuid} aria-label={`打开 ${playerName} 玩家中心`} title="打开玩家中心" onClick={() => player.puuid && onOpenPlayer(player.puuid)} className="min-w-0 truncate text-left text-[12px] font-bold text-white/85 hover:text-emerald-200 disabled:cursor-default">{playerName}</button>
          {tagLine && !privacy.enabled ? <span className="shrink-0 text-[10px] text-white/40">#{tagLine}</span> : null}
        </div>
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1 text-[10px] text-white/55"><span>{position && position !== "NONE" ? position : "未分配"}</span><span className="truncate text-white/40">{playerRankLabel(player)}</span>{playerLevel(player) != null ? <span className="text-white/35">等级 {playerLevel(player)}</span> : null}{player.premade_group ? <span className="rounded bg-violet-400/15 px-1 text-violet-200">组排 {String.fromCharCode(64 + Number(player.premade_group))}</span> : null}{player.tag?.label && !privacy.enabled ? <span className="rounded bg-amber-400/15 px-1 text-amber-100">{player.tag.label}</span> : null}</div>
      </div>
      <button type="button" aria-label={expanded ? `收起 ${playerName} 详情` : `展开 ${playerName} 详情`} aria-expanded={expanded} onClick={() => setExpanded((value) => !value)} className="grid h-7 w-7 shrink-0 place-items-center rounded border border-white/10 text-white/50 hover:bg-white/5 hover:text-white">{expanded ? <ChevronUp className="h-4 w-4"/> : <ChevronDown className="h-4 w-4"/>}</button>
    </div>
    <div className="grid grid-cols-3 gap-px border-b border-white/10 bg-white/5 text-center text-[10px]">
      <span className="bg-cs2-bg-elevated/90 px-1 py-1.5"><span className="block text-white/45">胜率</span><b className={Number(winRate) >= .53 ? "text-emerald-300" : Number(winRate) <= .47 ? "text-rose-300" : "text-white/85"}>{winRate == null ? "—" : `${Math.round(Number(winRate) * 100)}%`}</b></span>
      <span className="bg-cs2-bg-elevated/90 px-1 py-1.5"><span className="block text-white/45">KDA</span><b className="text-white/85">{averageKda == null ? "—" : Number(averageKda).toFixed(2)}</b></span>
      <span className="bg-cs2-bg-elevated/90 px-1 py-1.5"><span className="block text-white/45">Akari {akariScore == null ? "—" : Number(akariScore).toFixed(2)}</span><b className={outlier === "below" ? "text-rose-300" : "text-cyan-200"}>{akariScore == null ? "—" : Number(akariScore).toFixed(2)}</b></span>
    </div>
    <div className="min-h-0 flex-1 p-2">
      {data?.historical_preview ? <p className="mb-1 text-[10px] text-white/55">本局 {match.kills || 0}/{match.deaths || 0}/{match.assists || 0} · KDA {match.kda || 0} · 伤害 {match.damage || 0}</p> : <p className="mb-1 truncate text-[10px] text-white/55">{usageLine}</p>}
      {analysisTags.length ? <div className="mb-1 flex max-h-7 flex-wrap gap-1 overflow-hidden">{analysisTags.map((tag) => <em key={tag.id || tag.label} title={tag.title || tag.label} className={`rounded px-1.5 py-0.5 text-[9px] font-semibold not-italic ${TAG_TONES[tag.tone] || TAG_TONES.info}`}>{tag.label}</em>)}</div> : null}
      {championUsageRows.length ? <div className="mb-1 flex h-5 gap-1 overflow-hidden">{championUsageRows.slice(0, 9).map((champion, championIndex) => { const championId = Number(champion.champion_id ?? champion.championId ?? champion.id ?? 0); return <span key={`${championId}-${championIndex}`} title={champion.champion_name || champion.championName || `英雄 ${championId}`} className="h-5 w-5 shrink-0 overflow-hidden rounded border border-white/10 bg-white/5">{championId ? <img src={getLeagueChampionIconUrl(championId)} alt={champion.champion_name || champion.championName || "英雄"} className="h-full w-full object-cover"/> : null}</span>; })}</div> : null}
      {jungle.games_analyzed > 0 ? <p className="mb-1 line-clamp-2 text-[10px] leading-4 text-amber-200">打野画像：{jungle.draft}</p> : null}
      {historyPreview.length ? <div data-testid="ongoing-mini-history" className="min-h-0 space-y-0.5 overflow-hidden">{historyPreview.slice(0, 8).map((item) => <PlayerHistoryRow key={item.id} item={item} onPreviewGame={onPreviewGame} puuid={player.puuid}/>)}</div> : <div className="flex h-20 items-center justify-center rounded bg-white/[.03] text-[10px] text-white/40">{player.data_availability?.unavailable?.includes("history") ? "战绩不可用" : "暂无近期战绩"}</div>}
    </div>
    {expanded ? <PlayerDetails player={player} privacy={privacy} recentMatches={recentMatches} onOpenPlayer={onOpenPlayer} onError={onError}/> : null}
  </article>;
}

function TeamSection({ team, players, data, privacy, onOpenPlayer, onError, onPreviewGame }) {
  const meta = ongoingTeamMeta(team);
  const summary = teamSummary(team, players, data);
  return <section className={`rounded-2xl border bg-[#101114]/75 ${meta.border}`}>
    <header className="flex min-h-10 items-center gap-2 border-b border-white/10 px-4 py-2 text-sm font-bold">
      <span className={`h-2.5 w-2.5 rounded-full ${meta.dot}`}/><span className={meta.text}>{meta.label}</span>
      {summary.avgWinRate != null ? <span className={`text-xs ${summary.avgWinRate >= .5 ? "text-emerald-300" : "text-rose-300"}`}>{formatTeamStat(summary.avgWinRate * 100)}%</span> : null}
      {summary.avgKda != null ? <span className="text-xs text-white/65">· KDA {formatTeamStat(summary.avgKda, 2)}</span> : null}
      <span className="ml-auto text-[10px] font-normal text-white/40">{players.length} 名玩家</span>
    </header>
    <div className="grid justify-center gap-2 p-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
      {players.map((player, index) => <PlayerCard key={player.puuid || index} player={player} index={index} data={data} privacy={privacy} onOpenPlayer={onOpenPlayer} onError={onError} onPreviewGame={onPreviewGame}/>) }
    </div>
  </section>;
}

export default function LeagueOngoingGame({ streamerMode, useAliases, previewData = null, onExitPreview = () => {}, onOpenPlayer = () => {}, onPreviewGame = () => {}, onError = () => {} }) {
  const [data, setData] = useState(null);
  const [busy, setBusy] = useState(false);
  const [privacy, setPrivacy] = useState({ enabled: Boolean(streamerMode), aliases: Boolean(useAliases) });
  const requestInFlight = useRef(false);
  const requestSequence = useRef(0);
  const load = async () => {
    if (requestInFlight.current) return;
    requestInFlight.current = true;
    const sequence = ++requestSequence.current;
    setBusy(true);
    try {
      const [game, status] = await Promise.all([fetchLeagueOngoingGame(), fetchLeagueLabStatus()]);
      if (sequence !== requestSequence.current) return;
      setData(game);
      setPrivacy({ enabled: streamerMode ?? Boolean(status?.settings?.streamer_mode_enabled), aliases: useAliases ?? Boolean(status?.settings?.streamer_mode_use_aliases) });
    } catch (error) {
      onError(error?.response?.data?.detail || "实时对局读取失败");
    } finally {
      requestInFlight.current = false;
      setBusy(false);
    }
  };
  useEffect(() => {
    if (previewData) { setData(previewData); return undefined; }
    load();
    const timer = setInterval(load, 5000);
    return () => clearInterval(timer);
  }, [streamerMode, useAliases, previewData]);
  const grouped = (data?.players || []).reduce((out, row) => {
    const key = String(row.team || "未知队伍");
    (out[key] ??= []).push(row);
    return out;
  }, {});
  const orderedGroups = Object.entries(grouped).sort(([a], [b]) => {
    const order = { "TEAM-100": 0, "100": 0, TEAM_A: 0, "TEAM-200": 1, "200": 1, TEAM_B: 1, LOBBY: 0 };
    return (order[a] ?? 10) - (order[b] ?? 10) || a.localeCompare(b);
  });
  return <div data-testid="ongoing-game-root" className="mx-auto w-full max-w-[1500px] space-y-4 pb-6">
    <div className="flex items-center justify-between border-b border-white/10 pb-2"><div className="min-w-0"><h2 className="truncate font-bold"><Users className="mr-2 inline h-4 w-4"/>{data?.historical_preview ? `历史对局模拟 · Game ${data.game_id}` : queueMeta(data).title}</h2><p className="mt-1 truncate text-xs text-cs2-text-muted">{data?.historical_preview ? "只读重放历史阵容与结算数据，不会向客户端写入任何状态。" : data?.query_stage === "lobby" ? "房间阶段已开始分析当前队伍；进入英雄选择后会自动补全对手、英雄与分路。" : "读取当前 Gameflow 队伍，分析近期表现、当前英雄、组排关系和双方打野路线倾向。"}</p></div>{data?.historical_preview ? <button onClick={onExitPreview} className="rounded-xl border border-cs2-border px-3 py-2 text-xs">退出模拟</button> : <button onClick={load} className="rounded-xl border border-cs2-border px-3 py-2 text-xs"><RefreshCw className={`mr-1 inline h-4 w-4 ${busy ? "animate-spin" : ""}`}/>刷新</button>}</div>
    {!data?.available ? <div data-testid="ongoing-idle-state" className="flex min-h-[300px] flex-col items-center justify-center gap-3 rounded-2xl border border-dashed border-cs2-border bg-white/[.015] p-12 text-center text-sm text-cs2-text-muted"><span className="grid h-12 w-12 place-items-center rounded-full bg-white/[.04] text-xl">{data?.is_connected === false ? "↯" : data?.is_spectating ? "◌" : "⌁"}</span><span>{idleCopy(data)}</span><span className="text-[11px]">进入房间、英雄选择或游戏阶段后自动显示玩家</span></div> : null}
    {orderedGroups.map(([team, players]) => <TeamSection key={team} team={team} players={players} data={data} privacy={privacy} onOpenPlayer={onOpenPlayer} onError={onError} onPreviewGame={onPreviewGame}/>) }
  </div>;
}
