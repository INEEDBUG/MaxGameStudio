import { useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, Filter, RefreshCw, Search, SlidersHorizontal, Swords, Trophy, UserRound, Users, X } from "lucide-react";
import { getLeagueChampionIconUrl, getLeagueProfileIconUrl } from "../../api/api";
import { collectLeagueMatches } from "../../api/leagueLabApi";
import LeagueDetailedMatchCard from "./LeagueDetailedMatchCard";
import LeagueAdvancedMatchFilters from "./LeagueAdvancedMatchFilters";
import MatchPreviewer from "./MatchPreviewer";
import { formatLeagueTimestamp, leagueWinState, normalizeLeagueTimestamp } from "../../utils/leagueDisplay";
import { matchesLeagueRuleTree } from "../../utils/leagueMatchFilter";

const PAGE_SIZES = [10, 20, 40];
const EMPTY_FILTER_TREE = { type: "group", logic: "and", negate: false, children: [] };
const DEFAULT_COLLECT_SETTINGS = { countPerIteration: 20, expectedCount: 20, maxIteration: 20 };

function validPageSize(value) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 1 && parsed <= 100 ? parsed : 20;
}

const QUEUES = [
  ["all", "全部队列"],
  ["420", "单双排位"],
  ["440", "灵活排位"],
  ["450", "极地大乱斗"],
  ["430", "匹配对战"],
  ["400", "普通对战"],
  ["490", "快速游戏"],
];

function valueOf(row, ...keys) {
  for (const key of keys) {
    if (row?.[key] !== undefined && row?.[key] !== null && row?.[key] !== "") return row[key];
  }
  return null;
}

function matchTarget(match) {
  const targetPuuid = String(valueOf(match, "participant_puuid", "participantPuuid", "puuid") || "");
  const participants = Array.isArray(match?.participants) ? match.participants : [];
  return participants.find((row) => String(valueOf(row, "puuid", "participant_puuid") || "") === targetPuuid) || match;
}

function queueText(match) {
  const id = String(valueOf(match, "queue_id", "queueId") ?? "");
  return QUEUES.find(([value]) => value === id)?.[1] || valueOf(match, "game_mode", "gameMode", "game_type", "gameType") || "未知队列";
}

function matchResult(match) {
  const target = matchTarget(match);
  return leagueWinState(valueOf(target, "win", "result") ?? valueOf(match, "win", "result"));
}

function matchSearchText(match) {
  const participants = Array.isArray(match?.participants) ? match.participants : [];
  return [
    valueOf(match, "game_id", "gameId"),
    valueOf(match, "champion_name", "championName"),
    valueOf(match, "game_mode", "gameMode"),
    valueOf(match, "game_type", "gameType"),
    queueText(match),
    ...participants.flatMap((player) => [
      valueOf(player, "game_name", "gameName", "riot_id_game_name", "riotIdGameName"),
      valueOf(player, "tag_line", "tagLine", "riot_id_tagline", "riotIdTagline"),
    ]),
  ].filter(Boolean).join(" ").toLocaleLowerCase();
}

function getMatchKey(match, index) {
  return `${valueOf(match, "game_id", "gameId") || "unknown"}:${valueOf(match, "source") || "lcu"}:${index}`;
}

function numeric(value, fallback = 0) {
  const next = Number(value);
  return Number.isFinite(next) ? next : fallback;
}

function playerName(player, fallback = "未知玩家") {
  return String(valueOf(player, "game_name", "gameName", "riot_id_game_name", "riotIdGameName", "name") || fallback);
}

function profileIconId(player) {
  return valueOf(player, "profile_icon_id", "profileIconId", "profileIcon")
    ?? valueOf(player?.summoner, "profile_icon_id", "profileIconId", "profileIcon")
    ?? valueOf(player?.identity, "profile_icon_id", "profileIconId", "profileIcon");
}

function ProfileAvatar({ icon, alt = "玩家头像", className = "h-full w-full object-cover", fallbackClassName = "h-full w-full" }) {
  const [failed, setFailed] = useState(false);
  if (icon == null || failed) {
    return <span aria-label={alt} className={`grid place-items-center rounded-lg bg-black/30 text-white/50 ${fallbackClassName}`}><UserRound className="h-5 w-5" /></span>;
  }
  return <img src={getLeagueProfileIconUrl(icon)} alt={alt} className={className} onError={() => setFailed(true)} />;
}

function targetForHistory(match, currentPlayer) {
  const currentPuuid = String(currentPlayer?.puuid || "");
  const participants = Array.isArray(match?.participants) ? match.participants : [];
  if (currentPuuid) {
    const current = participants.find((row) => String(valueOf(row, "puuid", "participant_puuid") || "") === currentPuuid);
    if (current) return { ...match, ...current };
  }
  return matchTarget(match);
}

function historySummary(matches, currentPlayer) {
  const rows = (Array.isArray(matches) ? matches : []).map((match) => targetForHistory(match, currentPlayer));
  const wins = rows.filter((row) => leagueWinState(valueOf(row, "win", "result")) === true).length;
  const kda = rows.length
    ? rows.reduce((sum, row) => sum + (numeric(row.kills) + numeric(row.assists)) / Math.max(1, numeric(row.deaths)), 0) / rows.length
    : null;
  return { games: rows.length, wins, losses: Math.max(0, rows.length - wins), winRate: rows.length ? wins / rows.length : null, kda };
}

function masteryRows(matches, currentPlayer) {
  const map = new Map();
  for (const match of Array.isArray(matches) ? matches : []) {
    const row = targetForHistory(match, currentPlayer);
    const id = valueOf(row, "champion_id", "championId");
    const key = id == null ? String(valueOf(row, "champion_name", "championName") || "unknown") : String(id);
    const current = map.get(key) || { id, name: valueOf(row, "champion_name", "championName") || "未知英雄", games: 0, wins: 0 };
    current.games += 1;
    if (leagueWinState(valueOf(row, "win", "result")) === true) current.wins += 1;
    map.set(key, current);
  }
  return [...map.values()].sort((left, right) => right.games - left.games || right.wins - left.wins).slice(0, 5);
}

function relationshipRows(matches, currentPlayer) {
  const currentPuuid = String(currentPlayer?.puuid || "");
  const map = new Map();
  for (const match of Array.isArray(matches) ? matches : []) {
    const participants = Array.isArray(match?.participants) ? match.participants : [];
    const target = targetForHistory(match, currentPlayer);
    const team = valueOf(target, "team_id", "teamId") ?? valueOf(match, "team_id", "teamId");
    for (const row of participants) {
      const puuid = String(valueOf(row, "puuid", "participant_puuid") || "");
      if (!puuid || puuid === currentPuuid) continue;
      const key = `${puuid}:${String(valueOf(row, "team_id", "teamId") || "")}`;
      const current = map.get(key) || { ...row, puuid, games: 0 };
      current.games += 1;
      map.set(key, current);
    }
  }
  return [...map.values()].sort((left, right) => right.games - left.games).slice(0, 8);
}

// Adapted from LeagueAkari's MIT-licensed player-tab composition. The host
// keeps its existing React/API contracts and does not expose the upstream brand.
function HistorySidebar({ matches, currentPlayer, streamerMode, useAliases, onOpenPlayer }) {
  const summary = historySummary(matches, currentPlayer);
  const mastery = masteryRows(matches, currentPlayer);
  const relationships = relationshipRows(matches, currentPlayer);
  const historyProfile = targetForHistory(matches?.[0], currentPlayer) || {};
  // current_summoner is refreshed from gameflow and can briefly omit the
  // profile icon while the match-history row already contains it.  Merge only
  // defined values so the sidebar does not replace a real history avatar with
  // a transient null value.
  const profile = {
    ...historyProfile,
    ...Object.fromEntries(Object.entries(currentPlayer || {}).filter(([, value]) => value !== null && value !== undefined && value !== "")),
  };
  const name = streamerMode ? "当前玩家" : playerName(profile, "我的战绩");
  const tag = streamerMode ? "#####" : valueOf(profile, "tag_line", "tagLine", "riot_id_tagline", "riotIdTagline");
  const icon = profileIconId(currentPlayer) ?? profileIconId(historyProfile);
  const open = (puuid) => puuid && onOpenPlayer?.(puuid);
  return <aside data-testid="league-history-sidebar" className="space-y-3 lg:sticky lg:top-4">
    <section className="overflow-hidden rounded-xl border border-cs2-border bg-cs2-bg-elevated">
      <div className="relative h-20 bg-[radial-gradient(circle_at_80%_0%,rgba(34,211,238,.22),transparent_56%),linear-gradient(135deg,rgba(15,23,42,.92),rgba(20,20,22,.98))]" />
      <div className="relative z-10 -mt-8 px-4 pb-4">
        <div className="flex items-end gap-3">
          <div className="h-16 w-16 shrink-0 overflow-hidden rounded-lg border-2 border-cyan-200/70 bg-black/30 shadow-xl">
            <ProfileAvatar icon={icon} alt="玩家头像" fallbackClassName="h-full w-full" />
          </div>
          <div className="min-w-0 pb-1">
            <h3 className="truncate text-base font-bold text-cs2-text-primary">{name}</h3>
            <p className="truncate text-[11px] text-cs2-text-muted">{tag ? `#${tag}` : "当前客户端账号"}</p>
          </div>
        </div>
        <div className="mt-3 flex flex-wrap gap-1.5 text-[10px]">
          <span className="rounded bg-cyan-400/10 px-2 py-1 text-cyan-200">{summary.games} 场样本</span>
          {summary.winRate != null ? <span className={`rounded px-2 py-1 ${summary.winRate >= .5 ? "bg-emerald-400/10 text-emerald-200" : "bg-rose-400/10 text-rose-200"}`}>{Math.round(summary.winRate * 100)}% 胜率</span> : null}
        </div>
        {currentPlayer?.puuid ? <button type="button" onClick={() => open(currentPlayer.puuid)} className="mt-3 w-full rounded-lg border border-cyan-400/25 bg-cyan-400/10 px-3 py-2 text-xs font-semibold text-cyan-100 hover:bg-cyan-400/15">打开完整玩家中心</button> : null}
      </div>
    </section>

    <section className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3">
      <div className="mb-2 flex items-center gap-2 text-xs font-bold text-cs2-text-primary"><Trophy className="h-3.5 w-3.5 text-cyan-300" />近期总览</div>
      <div className="grid grid-cols-2 gap-1.5 text-[10px] text-cs2-text-muted">
        <span className="rounded bg-black/15 p-2">胜 / 负<strong className="mt-0.5 block text-sm text-cs2-text-primary">{summary.wins} / {summary.losses}</strong></span>
        <span className="rounded bg-black/15 p-2">平均 KDA<strong className="mt-0.5 block text-sm text-cs2-text-primary">{summary.kda == null ? "—" : summary.kda.toFixed(2)}</strong></span>
      </div>
    </section>

    <section className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3">
      <div className="mb-2 flex items-center gap-2 text-xs font-bold text-cs2-text-primary"><Swords className="h-3.5 w-3.5 text-violet-300" />英雄熟练度</div>
      {mastery.length ? <div className="space-y-1.5">{mastery.map((row, index) => <div key={`${row.id || row.name}-${index}`} className="flex items-center gap-2"><div className="h-7 w-7 shrink-0 overflow-hidden rounded">{row.id ? <img src={getLeagueChampionIconUrl(row.id)} alt="" className="h-full w-full object-cover" /> : <Swords className="m-1.5 h-4 w-4 text-white/30" />}</div><span className="min-w-0 flex-1 truncate text-[11px] text-cs2-text-secondary">{row.name}</span><span className="text-right text-[10px] text-cs2-text-muted">{row.games} 场<br /><b className="text-emerald-200">{row.games ? Math.round(row.wins / row.games * 100) : 0}%</b></span></div>)}</div> : <p className="text-[11px] text-cs2-text-muted">当前没有可用的英雄熟练度数据。</p>}
    </section>

    <section className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3">
      <div className="mb-2 flex items-center gap-2 text-xs font-bold text-cs2-text-primary"><Users className="h-3.5 w-3.5 text-amber-300" />近期共同对局</div>
      {relationships.length ? <div className="space-y-1">{relationships.map((row, index) => <button key={row.puuid || index} type="button" disabled={!row.puuid} onClick={() => open(row.puuid)} className="flex w-full items-center gap-2 rounded-lg px-1.5 py-1 text-left hover:bg-white/[.05] disabled:cursor-default"><div className="h-5 w-5 shrink-0 overflow-hidden rounded-full bg-black/20"><ProfileAvatar icon={profileIconId(row)} alt="共同对局玩家头像" fallbackClassName="h-full w-full" className="h-full w-full rounded-full object-cover" /></div><span className="min-w-0 flex-1 truncate text-[10px] text-cs2-text-secondary">{streamerMode ? `玩家 ${index + 1}` : playerName(row, `玩家 ${index + 1}`)}</span><span className="text-[10px] text-cs2-text-muted">{row.games} 场</span></button>)}</div> : <p className="text-[11px] text-cs2-text-muted">暂无可证明的共同对局关系。</p>}
    </section>
  </aside>;
}

function hasFilterTreePredicate(node) {
  if (!node || typeof node !== "object") return false;
  if (node.type === "rule") return Boolean(node.field && node.value !== "");
  return Array.isArray(node.children) && node.children.some(hasFilterTreePredicate);
}

function pageInfoValue(pageInfo, ...keys) {
  return valueOf(pageInfo || {}, ...keys);
}

function numberOrNull(value) {
  if (value == null || (typeof value === "string" && value.trim() === "")) return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function MatchSkeleton() {
  return <div data-testid="league-history-loading" className="space-y-2" aria-label="正在加载战绩">
    {[0, 1, 2].map((row) => <div key={row} className="h-32 animate-pulse rounded border border-cs2-border-subtle bg-white/[.035]" />)}
  </div>;
}

function EmptyState({ filtered, onClear }) {
  return <div data-testid="league-history-empty" className="flex min-h-52 flex-col items-center justify-center gap-3 rounded border border-dashed border-cs2-border-subtle bg-white/[.02] px-6 text-center">
    <span className="grid h-10 w-10 place-items-center rounded-full bg-white/[.05] text-cs2-text-muted"><Filter className="h-4 w-4" /></span>
    <p className="text-sm text-cs2-text-muted">{filtered ? "当前筛选条件下没有战绩" : "暂无可用战绩"}</p>
    {filtered ? <button type="button" onClick={onClear} className="rounded-lg border border-cs2-border px-3 py-1.5 text-xs text-cs2-text-secondary hover:text-cs2-text-primary">清空筛选</button> : null}
  </div>;
}

export default function LeagueMatchHistoryPanel({
  matches = [],
  busy = false,
  connected = false,
  onRefresh,
  pageInfo = null,
  onPageChange,
  streamerMode = false,
  useAliases = false,
  onOpenPlayer,
  onError,
  onDryRunGame,
  currentPlayer = null,
  pageSize: initialPageSize = 20,
  onPageSizeChange,
}) {
  const [query, setQuery] = useState("");
  const [queue, setQueue] = useState("all");
  const [result, setResult] = useState("all");
  const [filterMode, setFilterMode] = useState("simple");
  const [advancedTree, setAdvancedTree] = useState(EMPTY_FILTER_TREE);
  const [pageSize, setPageSize] = useState(validPageSize(initialPageSize));
  const [page, setPage] = useState(1);
  const [collectSettings, setCollectSettings] = useState(DEFAULT_COLLECT_SETTINGS);
  const [collectBusy, setCollectBusy] = useState(false);
  const [collectState, setCollectState] = useState(null);
  const [previewMatch, setPreviewMatch] = useState(null);

  useEffect(() => {
    setPageSize(validPageSize(initialPageSize));
  }, [initialPageSize]);

  useEffect(() => {
    const serverPage = numberOrNull(pageInfoValue(pageInfo, "page", "current_page", "currentPage"));
    if (serverPage != null && serverPage >= 1) setPage(serverPage);
    const serverPageSize = numberOrNull(pageInfoValue(pageInfo, "page_size", "pageSize"));
    if (serverPageSize != null && serverPageSize >= 1) setPageSize(validPageSize(serverPageSize));
  }, [pageInfo]);

  const filteredMatches = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return [...(Array.isArray(matches) ? matches : [])]
      .filter((match) => queue === "all" || String(valueOf(match, "queue_id", "queueId") ?? "") === queue)
      .filter((match) => result === "all" || (result === "win" ? matchResult(match) === true : matchResult(match) === false))
      .filter((match) => !normalizedQuery || matchSearchText(match).includes(normalizedQuery))
      .filter((match) => filterMode !== "advanced" || matchesLeagueRuleTree(match, advancedTree))
      .sort((left, right) => {
        const leftTime = normalizeLeagueTimestamp(valueOf(left, "played_at", "playedAt")) || 0;
        const rightTime = normalizeLeagueTimestamp(valueOf(right, "played_at", "playedAt")) || 0;
        return rightTime - leftTime;
      });
  }, [matches, query, queue, result, filterMode, advancedTree]);

  const hasServerPage = Boolean(pageInfo && (pageInfoValue(pageInfo, "page", "current_page", "currentPage") != null || pageInfoValue(pageInfo, "has_more", "hasMore") != null));
  const serverPageSize = validPageSize(pageInfoValue(pageInfo, "page_size", "pageSize") || pageSize);
  const currentPage = hasServerPage ? Math.max(1, numberOrNull(pageInfoValue(pageInfo, "page", "current_page", "currentPage")) || page) : Math.min(page, Math.max(1, Math.ceil(filteredMatches.length / pageSize)));
  // The LCU endpoint's `count` is the number of rows in this page, not the
  // total history size. Only use an explicit total when the server provides
  // one; otherwise report the visible page size below.
  const totalCount = numberOrNull(pageInfoValue(pageInfo, "total_count", "totalCount", "collection_count"));
  const serverPageCount = numberOrNull(pageInfoValue(pageInfo, "page_count", "pageCount", "total_pages", "totalPages"))
    || (totalCount != null ? Math.max(1, Math.ceil(totalCount / pageSize)) : null);
  const hasMore = Boolean(pageInfoValue(pageInfo, "has_more", "hasMore"));
  const pageCount = hasServerPage ? Math.max(currentPage, serverPageCount || (hasMore ? currentPage + 1 : currentPage)) : Math.max(1, Math.ceil(filteredMatches.length / pageSize));
  const visibleMatches = hasServerPage ? filteredMatches.slice(0, serverPageSize) : filteredMatches.slice((currentPage - 1) * pageSize, currentPage * pageSize);
  const filterActive = Boolean(query.trim() || queue !== "all" || result !== "all" || (filterMode === "advanced" && hasFilterTreePredicate(advancedTree)));

  useEffect(() => {
    setPage(1);
  }, [query, queue, result, pageSize]);

  const updatePageSize = (value) => {
    const next = Number(value);
    setPageSize(next);
    setPage(1);
    onPageSizeChange?.(next);
    onPageChange?.(1, next);
  };

  const goToPage = (nextPage) => {
    const next = Math.max(1, Math.min(pageCount, Number(nextPage) || 1));
    setPage(next);
    if (hasServerPage) onPageChange?.(next, pageSize);
  };

  const clearFilters = () => {
    setQuery("");
    setQueue("all");
    setResult("all");
    setAdvancedTree(EMPTY_FILTER_TREE);
  };

  const handleCollect = async () => {
    if (collectBusy || !connected || typeof collectLeagueMatches !== "function") return;
    setCollectBusy(true);
    setCollectState({ status: "running", expected_count: collectSettings.expectedCount, scanned_games_count: 0, count: 0, iterations: 0, source: "lcu" });
    try {
      const response = await collectLeagueMatches({
        count_per_iteration: Number(collectSettings.countPerIteration),
        expected_count: Number(collectSettings.expectedCount),
        max_iteration: Number(collectSettings.maxIteration),
        filter_tree: filterMode === "advanced" ? advancedTree : {},
        query: query.trim(),
        queue_id: queue === "all" ? null : Number(queue),
        result,
      });
      setCollectState({ ...(response || {}), status: "complete", source: response?.source || "lcu" });
      await onRefresh?.();
    } catch (error) {
      setCollectState({ status: "error", message: error?.response?.data?.detail || error?.message || "战绩收集失败" });
      onError?.(error?.response?.data?.detail || error?.message || "战绩收集失败");
    } finally {
      setCollectBusy(false);
    }
  };

  const source = String(pageInfoValue(pageInfo, "source") || (collectState?.source || valueOf(matches?.[0] || {}, "source")) || "lcu").toUpperCase();
  const collectProgress = collectState?.status === "running"
    ? `正在收集：已扫描 ${collectState.scanned_games_count || 0} 场，已匹配 ${collectState.count || 0} 场`
    : collectState?.status === "complete"
      ? `最近收集：匹配 ${collectState.count || 0} 场 · 扫描 ${collectState.scanned_games_count || 0} 场 · ${collectState.complete ? "已达到目标" : "扫描完成"}`
      : collectState?.status === "error" ? collectState.message : "支持按 LeagueAkari 规则收集并保存在本机。";

  return <section data-testid="league-match-history-panel" className="space-y-3">
    <div className="grid items-start gap-4 lg:grid-cols-[250px_minmax(0,1fr)]">
      <HistorySidebar matches={matches} currentPlayer={currentPlayer} streamerMode={streamerMode} useAliases={useAliases} onOpenPlayer={onOpenPlayer} />
      <main className="min-w-0 space-y-3">
    <div className="flex flex-wrap items-end justify-between gap-3">
      <div>
        <div className="inline-flex items-center gap-2 text-[11px] font-bold uppercase tracking-[.16em] text-cyan-300"><SlidersHorizontal className="h-3.5 w-3.5" /> match history</div>
        <h2 className="mt-1 text-lg font-bold text-cs2-text-primary">最近战绩</h2>
        <p className="mt-1 text-xs text-cs2-text-muted">按当前账号的时间顺序查看对局；点击右侧箭头展开完整对局卡片。</p>
      </div>
      <button type="button" disabled={!connected || busy} onClick={() => onRefresh?.()} className="inline-flex items-center gap-2 rounded-xl border border-cs2-border px-3 py-2 text-xs font-semibold text-cs2-text-secondary transition-colors hover:text-cs2-text-primary disabled:opacity-40"><RefreshCw className={`h-3.5 w-3.5 ${busy ? "animate-spin" : ""}`} />刷新战绩</button>
    </div>

    <div className="rounded border border-cs2-border bg-cs2-bg-elevated p-2">
      <div className="flex flex-wrap items-center gap-2">
        <label className="relative min-w-[200px] flex-1">
          <span className="sr-only">搜索战绩</span>
          <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-cs2-text-muted" />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索游戏 ID、英雄或玩家" className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input py-2 pl-9 pr-8 text-xs text-cs2-text-primary outline-none focus:border-cyan-400/50" />
          {query ? <button type="button" aria-label="清除战绩搜索" onClick={() => setQuery("")} className="absolute right-2 top-1/2 -translate-y-1/2 text-cs2-text-muted hover:text-cs2-text-primary"><X className="h-3.5 w-3.5" /></button> : null}
        </label>
        <select aria-label="战绩队列筛选" value={queue} onChange={(event) => setQueue(event.target.value)} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs text-cs2-text-secondary"><option value="all">全部队列</option>{QUEUES.slice(1).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select>
        <select aria-label="战绩结果筛选" value={result} onChange={(event) => setResult(event.target.value)} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs text-cs2-text-secondary"><option value="all">全部结果</option><option value="win">仅胜利</option><option value="loss">仅失败</option></select>
        <select aria-label="战绩每页数量" value={pageSize} onChange={(event) => updatePageSize(event.target.value)} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs text-cs2-text-secondary">{[...new Set([...PAGE_SIZES, pageSize])].sort((left, right) => left - right).map((size) => <option key={size} value={size}>每页 {size} 局</option>)}</select>
        {filterActive ? <button type="button" onClick={clearFilters} className="rounded-lg border border-cyan-400/30 bg-cyan-400/10 px-3 py-2 text-xs text-cyan-200">清空筛选</button> : null}
      </div>
      <div className="mt-2 flex flex-wrap items-center justify-between gap-2 px-1 text-[10px] text-cs2-text-muted"><span>共 {totalCount ?? filteredMatches.length} 场{filterActive ? " · 已筛选" : ""}</span><span className="inline-flex items-center gap-1.5"><span className="rounded border border-cyan-400/20 px-1.5 py-0.5 text-cyan-200">来源 {source}</span><span>比赛时间无法确认时显示“比赛时间未知”。</span></span></div>
    </div>

    <section data-testid="league-history-collect" className="rounded border border-violet-400/20 bg-violet-400/[.04] p-3">
      <div className="flex flex-wrap items-center justify-between gap-2"><div><b className="text-xs text-violet-100">本地收集模式</b><p className="mt-1 text-[10px] text-cs2-text-muted">按当前筛选向客户端分批读取并保存匹配战绩，不执行账号写入。</p></div><button type="button" aria-label="收集匹配战绩" disabled={!connected || busy || collectBusy} onClick={handleCollect} className="rounded-lg border border-violet-400/30 bg-violet-400/10 px-3 py-2 text-xs font-semibold text-violet-200 disabled:opacity-40">{collectBusy ? "正在收集…" : "收集更多战绩"}</button></div>
      <div className="mt-2 grid gap-2 sm:grid-cols-3"><label className="text-[10px] text-cs2-text-muted">每批<input aria-label="每批收集数量" type="number" min="1" max="100" value={collectSettings.countPerIteration} onChange={(event) => setCollectSettings((current) => ({ ...current, countPerIteration: Math.max(1, Math.min(100, Number(event.target.value) || 1)) }))} className="mt-1 block w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs text-cs2-text-primary" /></label><label className="text-[10px] text-cs2-text-muted">目标场数<input aria-label="收集目标数量" type="number" min="1" max="1000" value={collectSettings.expectedCount} onChange={(event) => setCollectSettings((current) => ({ ...current, expectedCount: Math.max(1, Math.min(1000, Number(event.target.value) || 1)) }))} className="mt-1 block w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs text-cs2-text-primary" /></label><label className="text-[10px] text-cs2-text-muted">最多批次<input aria-label="收集最多批次" type="number" min="1" max="100" value={collectSettings.maxIteration} onChange={(event) => setCollectSettings((current) => ({ ...current, maxIteration: Math.max(1, Math.min(100, Number(event.target.value) || 1)) }))} className="mt-1 block w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs text-cs2-text-primary" /></label></div>
      <div data-testid="league-history-collect-progress" className="mt-2 text-[10px] text-cs2-text-muted">{collectProgress}</div>
      {collectBusy ? <div className="mt-1 h-1 overflow-hidden rounded bg-white/10"><span className="block h-full w-1/2 animate-pulse rounded bg-violet-300" /></div> : null}
    </section>

    <div className="flex flex-wrap items-center gap-2 rounded border border-cs2-border-subtle bg-white/[.02] p-2"><span className="text-[10px] font-semibold text-cs2-text-secondary">高级筛选</span><button type="button" role="tab" aria-selected={filterMode === "simple"} onClick={() => setFilterMode("simple")} className={`rounded-lg px-2.5 py-1.5 text-[10px] ${filterMode === "simple" ? "bg-cyan-400/15 text-cyan-200" : "text-cs2-text-muted"}`}>简单</button><button type="button" role="tab" aria-selected={filterMode === "advanced"} onClick={() => setFilterMode("advanced")} className={`rounded-lg px-2.5 py-1.5 text-[10px] ${filterMode === "advanced" ? "bg-violet-400/15 text-violet-200" : "text-cs2-text-muted"}`}>AND / OR / NOT</button></div>
    {filterMode === "advanced" ? <LeagueAdvancedMatchFilters tree={advancedTree} onChange={setAdvancedTree} /> : null}

    {busy && matches.length === 0 ? <MatchSkeleton /> : null}
    {!busy && !visibleMatches.length ? <EmptyState filtered={filterActive} onClear={clearFilters} /> : null}
    {visibleMatches.length ? <div data-testid="league-history-list" className="space-y-2">{visibleMatches.map((match, index) => <div key={getMatchKey(match, index)} className="space-y-1"><div className="flex flex-wrap items-center gap-2 px-1 text-[10px] text-cs2-text-muted"><span className="font-semibold text-cs2-text-secondary">{queueText(match)}</span><span>·</span><span>{formatLeagueTimestamp(valueOf(match, "played_at", "playedAt"))}</span><span>·</span><span>对局 ID {valueOf(match, "game_id", "gameId") || "未知"}</span><button type="button" onClick={() => setPreviewMatch(match)} className="ml-auto rounded border border-cyan-400/25 px-2 py-1 text-cyan-200 hover:bg-cyan-400/10">独立预览</button></div><LeagueDetailedMatchCard match={match} streamerMode={streamerMode} useAliases={useAliases} onOpenPlayer={onOpenPlayer} onError={onError} onDryRunGame={onDryRunGame} /></div>)}</div> : null}

    <div className="flex flex-wrap items-center justify-between gap-2 rounded border border-cs2-border-subtle bg-white/[.02] px-3 py-2 text-xs text-cs2-text-muted">
      <span>第 {currentPage} / {pageCount} 页 · {visibleMatches.length} 场</span>
      <div className="flex items-center gap-1"><button type="button" aria-label="战绩第一页" disabled={currentPage <= 1 || busy} onClick={() => goToPage(1)} className="rounded-lg border border-cs2-border px-2 py-1.5 disabled:opacity-35">首</button><button type="button" aria-label="战绩上一页" disabled={currentPage <= 1 || busy} onClick={() => goToPage(currentPage - 1)} className="rounded-lg border border-cs2-border p-1.5 disabled:opacity-35"><ChevronLeft className="h-3.5 w-3.5" /></button><span className="min-w-10 text-center font-mono text-cs2-text-secondary">{currentPage}</span><button type="button" aria-label="战绩下一页" disabled={(hasServerPage ? !hasMore : currentPage >= pageCount) || busy} onClick={() => goToPage(currentPage + 1)} className="rounded-lg border border-cs2-border p-1.5 disabled:opacity-35"><ChevronRight className="h-3.5 w-3.5" /></button><button type="button" aria-label="战绩最后一页" disabled={hasServerPage || currentPage >= pageCount || busy} onClick={() => goToPage(pageCount)} className="rounded-lg border border-cs2-border px-2 py-1.5 disabled:opacity-35">末</button></div>
    </div>
      </main>
    </div>
    <MatchPreviewer show={Boolean(previewMatch)} match={previewMatch} streamerMode={streamerMode} useAliases={useAliases} onClose={() => setPreviewMatch(null)} onOpenPlayer={onOpenPlayer} onError={onError} onDryRunGame={onDryRunGame} />
  </section>;
}
