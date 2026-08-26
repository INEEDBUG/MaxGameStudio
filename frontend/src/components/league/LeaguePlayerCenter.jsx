import { useEffect, useMemo, useRef, useState } from "react";
import { Activity, ChevronLeft, ChevronRight, Clock3, MapPinned, RefreshCw, Search, Swords, Tag, Trophy, Users } from "lucide-react";
import { deleteLeaguePlayerSearchHistory, fetchCurrentLeaguePlayer, fetchLeaguePlayer, fetchLeaguePlayerCollection, fetchLeaguePlayerFriends, fetchLeaguePlayerJungleAnalysis, fetchLeaguePlayerSearchHistory, fetchLeaguePlayerSearchServers, fetchRecentLeaguePlayers, pinLeaguePlayerSearchHistory, saveLeaguePlayerTag, searchLeaguePlayer, spectateLeagueFriend } from "../../api/leagueLabApi";
import { getLeagueProfileIconUrl } from "../../api/api";
import LeagueMatchFilterPresets from "./LeagueMatchFilterPresets";
import LeagueAdvancedMatchFilters from "./LeagueAdvancedMatchFilters";
import LeagueMasteryCatalog from "./LeagueMasteryCatalog";
import LeagueEncounteredGames from "./LeagueEncounteredGames";
import LeagueDetailedMatchCard from "./LeagueDetailedMatchCard";
import LeagueChampionAnalysis from "./LeagueChampionAnalysis";
import LeaguePlayerSummary from "./LeaguePlayerSummary";
import LeaguePlayerSearchBrowser from "./LeaguePlayerSearchBrowser";
import { matchesLeagueRuleTree } from "../../utils/leagueMatchFilter";
import { leaguePrivacyText, maskLeagueName } from "../../utils/leagueStreamerMode";

const PLAYER_CENTER_MAX_WIDTH = "max-w-[1320px]";

function playerProfileIconId(summoner) {
  return summoner?.profile_icon_id ?? summoner?.profileIconId ?? summoner?.profileIcon;
}

function playerDisplayName(summoner, fallback = "未知玩家") {
  return String(summoner?.game_name || summoner?.gameName || summoner?.displayName || fallback);
}

function playerTagLine(summoner) {
  return String(summoner?.tag_line || summoner?.tagLine || "");
}

function queueRows(ranked) {
  if (Array.isArray(ranked?.queues)) return ranked.queues;
  if (ranked?.queueMap && typeof ranked.queueMap === "object") return Object.entries(ranked.queueMap).map(([queueType, row]) => ({ ...row, queueType }));
  return Object.values(ranked || {}).filter((row) => row && typeof row === "object" && (row.tier || row.division));
}

const TABS = [
  ["overview", "概览", Activity],
  ["history", "战绩", Clock3],
  ["champions", "英雄/熟练度", Swords],
  ["challenges", "挑战", Trophy],
  ["encounters", "遇到的对局", Users],
];

const EMPTY_FILTER_TREE = { type: "group", logic: "and", negate: false, children: [] };

function EmptyState({ children, testId }) {
  return <div data-testid={testId} className="rounded-xl border border-dashed border-cs2-border-subtle p-8 text-center text-xs text-cs2-text-muted">{children}</div>;
}

function ProfileTabs({ activeTab, onChange }) {
  return <nav data-testid="player-tabs" role="tablist" aria-label="玩家资料分区" className="flex gap-1 overflow-x-auto border-b border-cs2-border-subtle px-2 pt-2">
    {TABS.map(([id, label, Icon]) => <button key={id} id={`player-tab-${id}`} type="button" role="tab" aria-selected={activeTab === id} aria-controls={`player-panel-${id}`} onClick={() => onChange(id)} className={`inline-flex shrink-0 items-center gap-1.5 border-b-2 px-3 py-2.5 text-xs font-semibold transition-colors ${activeTab === id ? "border-cyan-300 text-cyan-200" : "border-transparent text-cs2-text-muted hover:border-white/20 hover:text-cs2-text-primary"}`}><Icon className="h-3.5 w-3.5"/>{label}</button>)}
  </nav>;
}

function RelationshipSummary({ matches, selfPuuid, streamerMode, useAliases, onOpen }) {
  const relationships = useMemo(() => {
    const groups = { teammates: new Map(), opponents: new Map() };
    for (const match of matches || []) {
      const participants = Array.isArray(match.participants) ? match.participants : [];
      const self = participants.find((row) => String(row.puuid || "") === String(selfPuuid || ""));
      if (!self) continue;
      const selfTeam = self.team_id;
      for (const row of participants) {
        const puuid = String(row.puuid || "");
        if (!puuid || puuid === String(selfPuuid || "")) continue;
        const group = String(row.team_id) === String(selfTeam) ? groups.teammates : groups.opponents;
        const current = group.get(puuid) || { ...row, puuid, games: 0, wins: 0 };
        current.games += 1;
        if (row.win === true) current.wins += 1;
        group.set(puuid, current);
      }
    }
    return { teammates: [...groups.teammates.values()].sort((a, b) => b.games - a.games).slice(0, 6), opponents: [...groups.opponents.values()].sort((a, b) => b.games - a.games).slice(0, 6) };
  }, [matches, selfPuuid]);

  const rowName = (row, index) => streamerMode ? maskLeagueName(row.game_name, index, useAliases, row.puuid) : (row.game_name || `玩家 ${index + 1}`);
  const group = (label, rows, tone) => <section className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3"><h4 className={`mb-2 text-[11px] font-bold ${tone}`}>{label}</h4>{rows.length ? <div className="space-y-1">{rows.map((row, index) => <button key={row.puuid} type="button" onClick={() => onOpen(row.puuid)} className="flex w-full items-center justify-between gap-2 rounded-lg px-2 py-1.5 text-left text-[11px] hover:bg-white/[.05]"><span className="min-w-0 truncate">{rowName(row, index)}</span><span className="shrink-0 text-cs2-text-muted">{row.games} 场 · {row.wins} 胜</span></button>)}</div> : <p className="text-[11px] text-cs2-text-muted">当前 bundle 没有可证明的共同对局关系。</p>}</section>;
  return <div data-testid="player-relationship-summary" className="space-y-3">{group("最近队友", relationships.teammates, "text-emerald-200")}{group("最近对手", relationships.opponents, "text-rose-200")}</div>;
}

function PlayerTagEditor({ tag, setTag, streamerMode, onSave }) {
  if (streamerMode) return <section className="rounded-xl border border-amber-400/20 bg-amber-400/[.05] p-3 text-[11px] leading-5 text-amber-100">直播隐私模式已隐藏本地玩家标签编辑器。</section>;
  return <section data-testid="player-local-tag" className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3"><div className="mb-2 text-xs font-semibold"><Tag className="mr-1 inline h-3.5 w-3.5"/>本地玩家标签</div><input value={tag.label} onChange={(event) => setTag({ ...tag, label: event.target.value })} placeholder="例如：擅长打野 / 可靠队友" className="mb-2 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><textarea value={tag.note} onChange={(event) => setTag({ ...tag, note: event.target.value })} placeholder="备注只保存在本机" className="h-20 w-full resize-none rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><button type="button" onClick={onSave} className="mt-2 rounded-lg bg-emerald-500 px-3 py-1.5 text-xs font-bold text-black">保存标签</button></section>;
}

export default function LeaguePlayerCenter({ accountPuuid = "", currentPuuid = "", streamerMode = false, useAliases = false, refreshSignal = 0, onLoadingChange, onError }) {
  const [query, setQuery] = useState("");
  const [data, setData] = useState(null);
  const [recent, setRecent] = useState([]);
  const [searchHistory, setSearchHistory] = useState([]);
  const [friends, setFriends] = useState([]);
  const [servers, setServers] = useState([]);
  const [selectedServer, setSelectedServer] = useState("");
  const [jungle, setJungle] = useState(null);
  const [jungleBusy, setJungleBusy] = useState(false);
  const [page, setPage] = useState(0);
  const [activeTab, setActiveTab] = useState("overview");
  const [filter, setFilter] = useState({ result: "all", mode: "all", position: "all", text: "", minKills: "", maxDeaths: "", minKda: "", advancedTree: EMPTY_FILTER_TREE });
  const [busy, setBusy] = useState(false);
  const [tag, setTag] = useState({ label: "", note: "", color: "emerald" });
  const loadRequest = useRef(0);
  const loadInFlight = useRef(null);
  const jungleRequest = useRef(0);
  const jungleInFlight = useRef(null);
  const disposed = useRef(false);
  const lastRefreshSignal = useRef(refreshSignal);

  const load = async (target = currentPuuid, nextPage = 0, collect = false, serverOverride) => {
    const trimmed = String(target || "").trim();
    const routeServer = serverOverride ?? selectedServer;
    const key = `${trimmed}|${nextPage}|${collect ? "collect" : "page"}|${routeServer || ""}`;
    if (loadInFlight.current?.key === key) return loadInFlight.current.promise;
    const request = ++loadRequest.current;
    setBusy(true);
    onLoadingChange?.(true);
    const promise = (async () => {
      try {
        let body;
        if (trimmed.includes("#")) {
          const splitAt = trimmed.lastIndexOf("#");
          body = await searchLeaguePlayer(trimmed.slice(0, splitAt), trimmed.slice(splitAt + 1), routeServer);
        } else if (trimmed) body = await fetchLeaguePlayer(trimmed, collect ? 100 : 20, collect ? 0 : nextPage * 20, routeServer);
        else body = await fetchCurrentLeaguePlayer();
        if (disposed.current || request !== loadRequest.current) return;
        const nextPuuid = String(body?.summoner?.puuid || "");
        if (nextPuuid && nextPuuid !== String(data?.summoner?.puuid || "")) setActiveTab("overview");
        setData(body && typeof body === "object" ? body : null);
        setQuery(`${body?.summoner?.game_name || ""}#${body?.summoner?.tag_line || ""}`);
        setPage(nextPage);
        setTag({ label: "", note: "", color: "emerald", ...(body?.tag || {}) });
        if (body?.server_id) setSelectedServer(body.server_id);
        try {
          const history = await fetchLeaguePlayerSearchHistory();
          if (!disposed.current && request === loadRequest.current) setSearchHistory(Array.isArray(history?.players) ? history.players : []);
        } catch { /* keep the current local list */ }
      } catch (error) {
        if (!disposed.current && request === loadRequest.current) onError?.(error?.response?.data?.detail || "玩家资料读取失败");
      } finally {
        if (!disposed.current && request === loadRequest.current) {
          setBusy(false);
          onLoadingChange?.(false);
        }
      }
    })();
    loadInFlight.current = { key, promise };
    try { return await promise; } finally { if (loadInFlight.current?.key === key) loadInFlight.current = null; }
  };
  const refreshRecent = async () => { try { setRecent((await fetchRecentLeaguePlayers()).players || []); } catch { setRecent([]); } };
  const refreshSearchHistory = async () => { try { setSearchHistory((await fetchLeaguePlayerSearchHistory()).players || []); } catch { setSearchHistory([]); } };
  const refreshFriends = async () => { try { setFriends((await fetchLeaguePlayerFriends()).friends || []); } catch { setFriends([]); } };
  const refreshServers = async () => { try { const body = await fetchLeaguePlayerSearchServers(); setServers(body.servers || []); setSelectedServer((value) => value || body.current || ""); } catch { setServers([]); } };
  const loadJungle = async (puuid, serverId = "") => {
    if (!puuid) return;
    const key = `${puuid}|${serverId || ""}`;
    if (jungleInFlight.current?.key === key) return jungleInFlight.current.promise;
    const request = ++jungleRequest.current;
    setJungleBusy(true);
    const promise = (async () => {
      try {
        const next = await fetchLeaguePlayerJungleAnalysis(puuid, 6, serverId);
        if (!disposed.current && request === jungleRequest.current) setJungle(next && typeof next === "object" ? next : null);
      } catch {
        if (!disposed.current && request === jungleRequest.current) setJungle(null);
      } finally {
        if (!disposed.current && request === jungleRequest.current) setJungleBusy(false);
      }
    })();
    jungleInFlight.current = { key, promise };
    try { return await promise; } finally { if (jungleInFlight.current?.key === key) jungleInFlight.current = null; }
  };
  const openCollection = async () => {
    const targetPuuid = data?.summoner?.puuid;
    if (!targetPuuid) return;
    const request = ++loadRequest.current;
    setBusy(true);
    try {
      const body = await fetchLeaguePlayerCollection(targetPuuid);
      if (disposed.current || request !== loadRequest.current) return;
      const matches = Array.isArray(body?.matches) ? body.matches : [];
      const count = Number(body?.count);
      setData({ ...data, matches, match_source: "sqlite", collection_count: Number.isFinite(count) ? count : matches.length, page: { beg_index: 0, end_index: Math.max(0, matches.length - 1), has_more: false } });
      setPage(0);
      setActiveTab("history");
    } catch (error) {
      if (!disposed.current && request === loadRequest.current) onError?.(error?.response?.data?.detail || "本地收集读取失败");
    } finally {
      if (!disposed.current && request === loadRequest.current) setBusy(false);
    }
  };
  useEffect(() => {
    disposed.current = false;
    return () => { disposed.current = true; loadRequest.current += 1; jungleRequest.current += 1; };
  }, []);
  useEffect(() => { void load(currentPuuid); void refreshRecent(); void refreshSearchHistory(); void refreshFriends(); void refreshServers(); }, [currentPuuid]);
  useEffect(() => {
    if (refreshSignal === lastRefreshSignal.current) return;
    lastRefreshSignal.current = refreshSignal;
    if (currentPuuid) void load(currentPuuid, 0);
  }, [refreshSignal, currentPuuid]);
  useEffect(() => { const puuid = data?.summoner?.puuid; if (puuid) void loadJungle(puuid, data?.server_id || selectedServer); else { jungleRequest.current += 1; setJungle(null); } }, [data?.summoner?.puuid, data?.server_id]);

  const rankedRows = useMemo(() => queueRows(data?.ranked), [data]);
  const masteryRows = useMemo(() => Array.isArray(data?.mastery) ? data.mastery : (data?.mastery?.championMasteries || []), [data]);
  const modes = useMemo(() => [...new Set((data?.matches || []).map((match) => match.game_mode).filter(Boolean))], [data]);
  const filteredMatches = useMemo(() => (data?.matches || []).filter((match) => {
    if (filter.result === "win" && match.win !== true) return false;
    if (filter.result === "loss" && match.win !== false) return false;
    if (filter.mode !== "all" && match.game_mode !== filter.mode) return false;
    if (filter.position !== "all" && String(match.position || "").toLowerCase() !== filter.position) return false;
    if (filter.minKills !== "" && Number(match.kills || 0) < Number(filter.minKills)) return false;
    if (filter.maxDeaths !== "" && Number(match.deaths || 0) > Number(filter.maxDeaths)) return false;
    const kda = (Number(match.kills || 0) + Number(match.assists || 0)) / Math.max(1, Number(match.deaths || 0));
    if (filter.minKda !== "" && kda < Number(filter.minKda)) return false;
    if (!matchesLeagueRuleTree(match, filter.advancedTree || EMPTY_FILTER_TREE)) return false;
    const text = filter.text.trim().toLowerCase();
    return !text || String(match.champion_name || "").toLowerCase().includes(text) || String(match.queue_id || "").includes(text);
  }), [data, filter]);
  const challengeSummary = useMemo(() => {
    const rows = data?.matches || [];
    const values = (key) => rows.map((row) => Number(row.challenges?.[key])).filter(Number.isFinite);
    const average = (key) => { const list = values(key); return list.length ? list.reduce((sum, value) => sum + value, 0) / list.length : null; };
    return { kda: average("kda"), killParticipation: average("killParticipation"), visionScorePerMinute: average("visionScorePerMinute"), damagePerMinute: average("damagePerMinute") };
  }, [data]);
  const collectionChallenges = useMemo(() => {
    const labels = { 505001: "已拥有英雄", 510001: "英雄皮肤", 510011: "炫彩皮肤", 504003: "守卫皮肤", 504002: "召唤师图标", 504004: "表情" };
    return (data?.player_challenges?.playerChallenges || []).filter((row) => labels[row.id]).map((row) => ({ ...row, label: labels[row.id] }));
  }, [data]);
  const summoner = data?.summoner || {};
  const viewerPuuid = accountPuuid || currentPuuid;
  const visibleName = streamerMode ? maskLeagueName(playerDisplayName(summoner), 0, useAliases, summoner.puuid) : playerDisplayName(summoner);
  const visibleTagLine = streamerMode ? "#####" : playerTagLine(summoner);
  const profileIconId = playerProfileIconId(summoner);
  const totalMatches = Number(data?.page?.total_count ?? data?.total_count ?? data?.collection_count ?? data?.matches?.length ?? 0);
  const summaryMetrics = useMemo(() => {
    const rows = Array.isArray(data?.matches) ? data.matches : [];
    const wins = rows.filter((row) => row.win === true).length;
    const kdaValues = rows.map((row) => (Number(row.kills || 0) + Number(row.assists || 0)) / Math.max(1, Number(row.deaths || 0)));
    const average = (key) => {
      const values = rows.map((row) => Number(row[key])).filter(Number.isFinite);
      return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : null;
    };
    return {
      games: totalMatches || rows.length,
      wins,
      losses: Math.max(0, rows.length - wins),
      winRate: rows.length ? wins / rows.length : null,
      kda: kdaValues.length ? kdaValues.reduce((sum, value) => sum + value, 0) / kdaValues.length : null,
      killParticipation: average("kill_participation") ?? average("killParticipation"),
      damageShare: average("damage_share") ?? average("damageShare") ?? average("champion_damage_percentage_of_team"),
    };
  }, [data, totalMatches]);
  const applyFilterPreset = (next) => setFilter({ ...next, advancedTree: next?.advancedTree || { type: "group", logic: next?.advancedLogic || "and", negate: false, children: (next?.advancedRules || []).map((rule) => ({ type: "rule", scope: "self", ...rule })) } });
  const pinSearchHistory = async (puuid, pinned, serverId) => { try { await pinLeaguePlayerSearchHistory(puuid, pinned, serverId); await refreshSearchHistory(); } catch (error) { onError?.(error?.response?.data?.detail || "最近访问置顶失败"); } };
  const deleteSearchHistory = async (puuid, serverId) => { try { await deleteLeaguePlayerSearchHistory(puuid, serverId); await refreshSearchHistory(); } catch (error) { onError?.(error?.response?.data?.detail || "删除最近访问失败"); } };
  const spectateFriend = async (puuid) => { try { await spectateLeagueFriend(puuid); } catch (error) { onError?.(error?.response?.data?.detail || "启动观战失败"); } };
  const saveTag = async () => { try { await saveLeaguePlayerTag(summoner.puuid, tag); } catch (error) { onError?.(error?.response?.data?.detail || "保存标签失败"); } };
  const resetFilters = () => setFilter({ result: "all", mode: "all", position: "all", text: "", minKills: "", maxDeaths: "", minKda: "", advancedTree: EMPTY_FILTER_TREE });

  const formatJunglePercent = (value) => { const number = Number(value); return Number.isFinite(number) ? `${Math.round(number * 100)}%` : "—"; };
  const renderJungle = () => (jungleBusy || jungle) ? <section className="rounded-2xl border border-emerald-400/20 bg-emerald-400/[.05] p-4"><div className="flex items-center justify-between"><h3 className="text-sm font-bold"><MapPinned className="mr-1 inline h-4 w-4 text-emerald-300"/>打野路线画像</h3><button type="button" disabled={jungleBusy} onClick={() => loadJungle(summoner.puuid, data.server_id || selectedServer)} className="rounded-lg border border-emerald-400/20 px-2 py-1 text-[10px] disabled:opacity-50"><RefreshCw className={`mr-1 inline h-3 w-3 ${jungleBusy ? "animate-spin" : ""}`}/>重算</button></div>{jungleBusy && !jungle ? <p className="mt-3 text-xs text-cs2-text-muted">正在读取最近时间线…</p> : jungle?.games_analyzed ? <><div className="mt-3 grid gap-2 sm:grid-cols-3"><span className="rounded-lg bg-black/10 p-3 text-xs">上半区活动<br/><b className="text-base">{formatJunglePercent(jungle.zone_percentages?.top)}</b></span><span className="rounded-lg bg-black/10 p-3 text-xs">中路活动<br/><b className="text-base">{formatJunglePercent(jungle.zone_percentages?.mid)}</b></span><span className="rounded-lg bg-black/10 p-3 text-xs">下半区活动<br/><b className="text-base">{formatJunglePercent(jungle.zone_percentages?.bot)}</b></span></div><p className="mt-3 rounded-lg border border-emerald-400/15 bg-black/10 p-3 text-xs leading-5 text-emerald-100">{jungle.draft}</p><p className="mt-2 text-[10px] text-cs2-text-muted">基于最近 {jungle.games_analyzed} 场可用打野时间线 · {String(jungle.history_source || "lcu").toUpperCase()} 数据源 · 仅生成草稿，不自动发送</p></> : <p className="mt-3 text-xs text-cs2-text-muted">{jungle?.reason || "最近战绩中没有可用的打野时间线"}</p>}</section> : null;

  const renderHistory = () => <section id="player-panel-history" role="tabpanel" aria-labelledby="player-tab-history" data-testid="player-history-panel" className="space-y-3"><LeagueMatchFilterPresets filter={filter} onApply={applyFilterPreset}/><LeagueAdvancedMatchFilters tree={filter.advancedTree} onChange={(advancedTree) => setFilter({ ...filter, advancedTree })}/><div className="grid gap-2 md:grid-cols-4"><select value={filter.position} onChange={(event) => setFilter({ ...filter, position: event.target.value })} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"><option value="all">全部位置</option><option value="top">上路</option><option value="jungle">打野</option><option value="middle">中路</option><option value="bottom">下路</option><option value="utility">辅助</option></select><input type="number" min="0" value={filter.minKills} onChange={(event) => setFilter({ ...filter, minKills: event.target.value })} placeholder="最少击杀" className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><input type="number" min="0" value={filter.maxDeaths} onChange={(event) => setFilter({ ...filter, maxDeaths: event.target.value })} placeholder="最多死亡" className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><div className="flex gap-2"><input type="number" min="0" step="0.1" value={filter.minKda} onChange={(event) => setFilter({ ...filter, minKda: event.target.value })} placeholder="最低 KDA" className="min-w-0 flex-1 rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><button type="button" onClick={resetFilters} className="rounded-xl border border-cs2-border px-3 py-2 text-xs">清空</button></div></div><div className="grid gap-2 md:grid-cols-[1fr_auto_auto]"><input value={filter.text} onChange={(event) => setFilter({ ...filter, text: event.target.value })} placeholder="筛选英雄或队列 ID" className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><select value={filter.result} onChange={(event) => setFilter({ ...filter, result: event.target.value })} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"><option value="all">全部结果</option><option value="win">仅胜利</option><option value="loss">仅失败</option></select><select value={filter.mode} onChange={(event) => setFilter({ ...filter, mode: event.target.value })} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"><option value="all">全部模式</option>{modes.map((mode) => <option key={mode} value={mode}>{mode}</option>)}</select></div><div className="space-y-3">{filteredMatches.map((match,index) => <LeagueDetailedMatchCard key={match.game_id || `match-${match.played_at || "unknown"}-${index}`} match={match} streamerMode={streamerMode} useAliases={useAliases} onOpenPlayer={(puuid) => load(puuid, 0, false, data.server_id || selectedServer)} onError={onError}/>)}{!filteredMatches.length && <EmptyState testId="player-history-empty">当前筛选条件下没有战绩</EmptyState>}</div><div className="flex justify-end gap-2"><button type="button" disabled={page === 0 || busy} onClick={() => load(summoner.puuid, page - 1, false, data.server_id || selectedServer)} className="rounded-lg border border-cs2-border px-3 py-2 text-xs disabled:opacity-40"><ChevronLeft className="inline h-3.5 w-3.5"/> 上一页</button><span className="px-2 py-2 text-xs text-cs2-text-muted">第 {page + 1} 页</span><button type="button" disabled={!data.page?.has_more || busy} onClick={() => load(summoner.puuid, page + 1, false, data.server_id || selectedServer)} className="rounded-lg border border-cs2-border px-3 py-2 text-xs disabled:opacity-40">下一页 <ChevronRight className="inline h-3.5 w-3.5"/></button></div></section>;

  return <div data-testid="player-center-root" className={`mx-auto w-full ${PLAYER_CENTER_MAX_WIDTH} space-y-4 pb-6`}>
    {!data ? <EmptyState testId="player-loading">正在读取玩家资料…</EmptyState> : null}
    <div className="grid gap-2 md:grid-cols-[minmax(220px,1fr)_180px_auto_auto_auto]"><div className="relative"><Search className="absolute left-3 top-2.5 h-4 w-4 text-cs2-text-muted"/><input value={streamerMode ? "" : query} disabled={streamerMode} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => event.key === "Enter" && load(query, 0)} placeholder={streamerMode ? "直播隐私模式已隐藏 Riot ID 搜索框" : "搜索 Riot ID，例如：玩家名#标签"} className="w-full rounded-xl border border-cs2-border bg-cs2-bg-input py-2 pl-9 pr-3 text-sm disabled:opacity-60"/></div><select aria-label="搜索区服" value={selectedServer} disabled={streamerMode} onChange={(event) => setSelectedServer(event.target.value)} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs disabled:opacity-60"><option value="">当前客户端区服</option>{servers.map((server) => <option key={server.id} value={server.id}>{server.label}{server.current ? "（当前）" : ""}</option>)}</select><button type="button" disabled={streamerMode} onClick={() => load(query, 0)} className="rounded-xl border border-cs2-border px-4 text-xs font-semibold disabled:opacity-40"><RefreshCw className={`inline h-4 w-4 ${busy ? "animate-spin" : ""}`}/> 读取</button><button type="button" disabled={!data?.summoner?.puuid || busy} onClick={() => load(data.summoner.puuid, 0, true, data.server_id || selectedServer)} className="rounded-xl border border-cyan-400/30 bg-cyan-400/10 px-4 text-xs font-semibold text-cyan-200 disabled:opacity-40">收集 100 场</button><button type="button" disabled={!data?.collection_count || busy} onClick={openCollection} className="rounded-xl border border-violet-400/30 bg-violet-400/10 px-4 text-xs font-semibold text-violet-200 disabled:opacity-40">本地 {data?.collection_count || 0} 场</button></div>
    <LeaguePlayerSearchBrowser history={searchHistory} friends={friends} streamerMode={streamerMode} useAliases={useAliases} onOpen={(puuid, serverId) => load(puuid, 0, false, serverId)} onPin={pinSearchHistory} onDelete={deleteSearchHistory} onSpectate={spectateFriend}/>
    {data && <div data-testid="player-profile-shell" className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_280px]">
      <main className="min-w-0 space-y-4">
        <section data-testid="player-profile-header" className="overflow-hidden rounded-2xl border border-cs2-border bg-cs2-bg-elevated"><div className="grid gap-4 p-5 md:grid-cols-[minmax(0,1fr)_auto]">
          <div className="flex min-w-0 items-center gap-3">
            <div className="relative h-16 w-16 shrink-0 overflow-hidden rounded-lg bg-white/[.05]">
              {profileIconId != null ? <img src={getLeagueProfileIconUrl(profileIconId)} alt="玩家头像" className="h-full w-full object-cover"/> : <span className="grid h-full w-full place-items-center text-xl text-cs2-text-muted">?</span>}
              <span className="absolute bottom-0 right-0 rounded bg-black/70 px-1.5 py-0.5 text-[10px] text-white">{summoner.summoner_level || "—"}</span>
            </div>
            <div className="min-w-0">
              <div className="truncate text-xl font-bold">{visibleName}{!streamerMode && <span className="ml-1 text-sm font-normal text-cs2-text-muted">#{visibleTagLine || "—"}</span>}</div>
              <div className="mt-1 text-xs text-cs2-text-muted">{leaguePrivacyText(summoner.puuid, streamerMode)} · {String(data.server_id || "").toUpperCase() || "当前客户端区服"}</div>
              <div className="mt-3 flex flex-wrap gap-1.5 text-[10px] text-cs2-text-muted"><span className="rounded border border-cs2-border-subtle bg-white/[.03] px-2 py-1">{summaryMetrics.games ? `近 ${summaryMetrics.games} 场` : "暂无战绩"}</span>{summaryMetrics.winRate != null ? <span className={`rounded border px-2 py-1 ${summaryMetrics.winRate >= .5 ? "border-emerald-400/30 text-emerald-200" : "border-rose-400/30 text-rose-200"}`}>{Math.round(summaryMetrics.winRate * 100)}% 胜率</span> : null}{summaryMetrics.kda != null ? <span className="rounded border border-cyan-400/20 px-2 py-1 text-cyan-200">KDA {summaryMetrics.kda.toFixed(2)}</span> : null}</div>
            </div>
          </div>
          <div className="flex items-start justify-end gap-2"><button type="button" aria-label="刷新玩家资料" title="刷新玩家资料" disabled={busy} onClick={() => load(summoner.puuid, 0, false, data.server_id || selectedServer)} className="rounded-lg border border-cs2-border px-3 py-2 text-xs disabled:opacity-40"><RefreshCw className={`mr-1 inline h-3.5 w-3.5 ${busy ? "animate-spin" : ""}`}/>刷新</button><div className="hidden text-right text-[11px] text-cs2-text-muted sm:block"><span>{data.matches?.length ? `已读取 ${data.matches.length} 场` : "暂无已读取战绩"}</span><br/><span className="text-[10px]">{String(data.match_source || "lcu").toUpperCase()} 数据源</span></div></div>
        </div><ProfileTabs activeTab={activeTab} onChange={setActiveTab}/></section>
        {activeTab === "overview" && <section id="player-panel-overview" role="tabpanel" aria-labelledby="player-tab-overview" data-testid="player-overview-panel" className="space-y-4">{data.matches?.length ? <LeaguePlayerSummary matches={data.matches}/> : <EmptyState testId="player-overview-empty">当前 payload 没有可计算的近期战绩摘要。</EmptyState>}{renderJungle()}</section>}
        {activeTab === "history" && renderHistory()}
        {activeTab === "champions" && <section id="player-panel-champions" role="tabpanel" aria-labelledby="player-tab-champions" data-testid="player-champions-panel" className="space-y-4"><section>{masteryRows.length ? <LeagueMasteryCatalog puuid={summoner.puuid} initialRows={masteryRows} onError={onError}/> : <EmptyState testId="player-mastery-empty">当前 bundle 没有英雄熟练度数据。</EmptyState>}</section>{data.matches?.length ? <LeagueChampionAnalysis matches={data.matches}/> : <EmptyState testId="player-champion-empty">当前 bundle 没有足够的战绩用于英雄分析。</EmptyState>}</section>}
        {activeTab === "challenges" && <section id="player-panel-challenges" role="tabpanel" aria-labelledby="player-tab-challenges" data-testid="player-challenges-panel" className="space-y-4">{collectionChallenges.length ? <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><h3 className="mb-3 text-sm font-bold">藏品挑战</h3><div className="grid grid-cols-2 gap-2 md:grid-cols-3">{collectionChallenges.map((row) => <span key={row.id} className="rounded-lg bg-white/[.04] p-3 text-xs">{row.label}<br/><b className="text-base">{Number(row.currentValue || 0).toLocaleString()}</b><span className="ml-2 text-[10px] text-cs2-text-muted">{row.currentLevel || ""}</span></span>)}</div></section> : <EmptyState testId="player-collection-empty">当前 payload 没有可识别的藏品挑战数据。</EmptyState>}{Object.values(challengeSummary).some((value) => value != null) ? <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><h3 className="mb-3 text-sm font-bold">近期挑战指标</h3><div className="grid grid-cols-2 gap-2 md:grid-cols-4">{challengeSummary.kda != null && <span className="rounded-lg bg-white/[.04] p-3 text-xs">平均 KDA<br/><b className="text-base">{challengeSummary.kda.toFixed(2)}</b></span>}{challengeSummary.killParticipation != null && <span className="rounded-lg bg-white/[.04] p-3 text-xs">参团率<br/><b className="text-base">{(challengeSummary.killParticipation * 100).toFixed(0)}%</b></span>}{challengeSummary.visionScorePerMinute != null && <span className="rounded-lg bg-white/[.04] p-3 text-xs">每分钟视野<br/><b className="text-base">{challengeSummary.visionScorePerMinute.toFixed(2)}</b></span>}{challengeSummary.damagePerMinute != null && <span className="rounded-lg bg-white/[.04] p-3 text-xs">每分钟伤害<br/><b className="text-base">{challengeSummary.damagePerMinute.toFixed(0)}</b></span>}</div></section> : <EmptyState testId="player-challenge-empty">当前战绩 payload 没有可计算的挑战指标。</EmptyState>}</section>}
        {activeTab === "encounters" && <section id="player-panel-encounters" role="tabpanel" aria-labelledby="player-tab-encounters" data-testid="player-encounters-panel" className="space-y-3"><LeagueEncounteredGames puuid={summoner.puuid} selfPuuid={viewerPuuid} onError={onError} emptyLabel="暂无可证明的共同对局；该玩家的 encounters 数据为空。"/></section>}
        <div className="flex justify-end gap-2"><span className="rounded-md border border-amber-400/20 bg-amber-400/10 px-2 py-1 text-[10px] font-semibold text-amber-200">排位源：{String(data.ranked_source || "none").toUpperCase()}</span><span className="rounded-md border border-cyan-400/20 bg-cyan-400/10 px-2 py-1 text-[10px] font-semibold text-cyan-200">战绩源：{String(data.match_source || "lcu").toUpperCase()}</span></div>
      </main>
      <aside data-testid="player-profile-sidebar" className="self-start space-y-3 lg:sticky lg:top-4"><section data-testid="player-sidebar-summary" className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3"><h4 className="mb-2 text-[11px] font-bold text-cs2-text-secondary">战绩摘要</h4><div className="grid grid-cols-2 gap-1.5"><span className="rounded bg-black/10 p-2 text-[10px] text-cs2-text-muted">胜 / 负<strong className="mt-0.5 block text-sm text-cs2-text-primary">{summaryMetrics.wins} / {summaryMetrics.losses}</strong></span><span className="rounded bg-black/10 p-2 text-[10px] text-cs2-text-muted">参团率<strong className="mt-0.5 block text-sm text-cs2-text-primary">{summaryMetrics.killParticipation == null ? "—" : `${Math.round(summaryMetrics.killParticipation * 100)}%`}</strong></span><span className="rounded bg-black/10 p-2 text-[10px] text-cs2-text-muted">团队伤害<strong className="mt-0.5 block text-sm text-cs2-text-primary">{summaryMetrics.damageShare == null ? "—" : `${Math.round(summaryMetrics.damageShare * 100)}%`}</strong></span><span className="rounded bg-black/10 p-2 text-[10px] text-cs2-text-muted">数据源<strong className="mt-0.5 block text-sm text-cs2-text-primary">{String(data.match_source || "lcu").toUpperCase()}</strong></span></div></section><PlayerTagEditor tag={tag} setTag={setTag} streamerMode={streamerMode} onSave={saveTag}/><RelationshipSummary matches={data.matches || []} selfPuuid={viewerPuuid} streamerMode={streamerMode} useAliases={useAliases} onOpen={(puuid) => load(puuid, 0, false, data.server_id || selectedServer)}/><section className="rounded-xl border border-cs2-border-subtle bg-white/[.02] p-3"><h4 className="mb-2 text-[11px] font-bold text-cs2-text-secondary">最近访问摘要</h4>{recent.length ? <div className="space-y-1">{recent.slice(0, 5).map((row, index) => <button key={row.puuid} type="button" onClick={() => load(row.puuid, 0)} className="flex w-full justify-between gap-2 rounded-lg px-2 py-1.5 text-left text-[11px] hover:bg-white/[.05]"><span className="truncate">{streamerMode ? maskLeagueName(row.game_name, index, useAliases, row.puuid) : (row.game_name || "未知玩家")}</span><span className="shrink-0 text-cs2-text-muted">{row.last_game_id ? "有最近对局" : ""}</span></button>)}</div> : <p className="text-[11px] text-cs2-text-muted">暂无最近访问记录。</p>}</section></aside>
    </div>}
  </div>;
}
