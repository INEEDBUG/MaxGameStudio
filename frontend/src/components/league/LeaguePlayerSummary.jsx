import { Activity, Coins, Crosshair, Eye, Shield, Trophy } from "lucide-react";
import { computeAkariScore } from "./LeagueChampionAnalysis";

const number = (value) => Number.isFinite(Number(value)) ? Number(value) : 0;
const average = (rows, getter) => rows.length ? rows.reduce((sum, row) => sum + number(getter(row)), 0) / rows.length : 0;

function teamRows(match) {
  return (match.participants || []).filter((row) => String(row.team_id) === String(match.team_id));
}

function share(match, key) {
  const total = teamRows(match).reduce((sum, row) => sum + number(row[key]), 0);
  return total > 0 ? number(match[key]) / total : 0;
}

function timestamp(value) {
  if (value == null) return 0;
  if (typeof value === "number") return value < 1e12 ? value * 1000 : value;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function aggregatePlayerSummary(matches = [], now = Date.now()) {
  if (!matches.length) return null;
  const kills = matches.reduce((sum, row) => sum + number(row.kills), 0);
  const deaths = matches.reduce((sum, row) => sum + number(row.deaths), 0);
  const assists = matches.reduce((sum, row) => sum + number(row.assists), 0);
  const wins = matches.filter((row) => row.win).length;
  const sorted = [...matches].sort((a, b) => timestamp(b.played_at) - timestamp(a.played_at));
  const newest = timestamp(sorted[0]?.played_at);
  const active = [];
  if (newest && now - newest <= 4 * 60 * 60 * 1000) {
    for (const match of sorted) {
      if (active.length && timestamp(active.at(-1).played_at) - timestamp(match.played_at) > 8 * 60 * 60 * 1000) break;
      active.push(match);
    }
  }
  const firstResult = Boolean(sorted[0]?.win);
  let streak = 0;
  for (const match of sorted) {
    if (Boolean(match.win) !== firstResult) break;
    streak += 1;
  }
  const killParticipation = average(matches, (match) => {
    const teamKills = teamRows(match).reduce((sum, row) => sum + number(row.kills), 0);
    return (number(match.kills) + number(match.assists)) / Math.max(1, teamKills);
  });
  return {
    games: matches.length,
    wins,
    losses: matches.length - wins,
    winRate: wins / matches.length,
    kda: (kills + assists) / Math.max(1, deaths),
    averageLine: `${(kills / matches.length).toFixed(1)} / ${(deaths / matches.length).toFixed(1)} / ${(assists / matches.length).toFixed(1)}`,
    killParticipation,
    damageShare: average(matches, (match) => share(match, "damage")),
    damageTakenShare: average(matches, (match) => share(match, "damage_taken")),
    goldShare: average(matches, (match) => share(match, "gold")),
    csPerMinute: average(matches, (match) => number(match.cs) / Math.max(1, number(match.duration_seconds) / 60)),
    blueSide: matches.filter((match) => Number(match.team_id) === 100).length,
    redSide: matches.filter((match) => Number(match.team_id) === 200).length,
    streak: streak >= 2 ? { count: streak, winning: firstResult } : null,
    activeSession: active.length ? { games: active.length, wins: active.filter((match) => match.win).length } : null,
    akariScore: computeAkariScore(matches),
  };
}

function Stat({ icon: Icon, label, value, detail, tone = "text-cs2-text-primary" }) {
  return <span className="rounded-xl border border-cs2-border-subtle bg-white/[.025] p-3 text-xs text-cs2-text-muted"><Icon className="mb-2 h-4 w-4 text-cyan-300"/><b className={`block text-lg ${tone}`}>{value}</b>{label}{detail ? <small className="mt-1 block text-[10px]">{detail}</small> : null}</span>;
}

export default function LeaguePlayerSummary({ matches = [] }) {
  const summary = aggregatePlayerSummary(matches);
  if (!summary) return null;
  const scoreTone = summary.akariScore.extraordinary ? "text-violet-300" : summary.akariScore.outstanding ? "text-emerald-300" : "text-cs2-text-primary";
  return <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
    <div className="mb-3 flex flex-wrap items-center justify-between gap-2"><div><h3 className="text-sm font-bold">玩家综合摘要</h3><p className="mt-1 text-[10px] text-cs2-text-muted">基于当前读取的 {summary.games} 场本地计算；与 LeagueAkari 玩家摘要口径一致。</p></div>{summary.streak ? <span className={`rounded-lg px-2 py-1 text-xs font-bold ${summary.streak.winning ? "bg-emerald-400/10 text-emerald-200" : "bg-rose-400/10 text-rose-200"}`}>{summary.streak.count} 连{summary.streak.winning ? "胜" : "败"}</span> : null}</div>
    <div className="grid grid-cols-2 gap-2 md:grid-cols-4 lg:grid-cols-5"><Stat icon={Trophy} label="Akari Score" value={summary.akariScore.total.toFixed(2)} detail={`${summary.akariScore.maxScore} 分制`} tone={scoreTone}/><Stat icon={Activity} label="平均 KDA" value={summary.kda.toFixed(2)} detail={summary.averageLine}/><Stat icon={Trophy} label="胜负" value={`${summary.wins} / ${summary.losses}`} detail={`胜率 ${Math.round(summary.winRate * 100)}%`}/><Stat icon={Crosshair} label="平均参团" value={`${Math.round(summary.killParticipation * 100)}%`}/><Stat icon={Activity} label="每分钟补刀" value={summary.csPerMinute.toFixed(1)}/><Stat icon={Crosshair} label="团队伤害占比" value={`${Math.round(summary.damageShare * 100)}%`}/><Stat icon={Shield} label="团队承伤占比" value={`${Math.round(summary.damageTakenShare * 100)}%`}/><Stat icon={Coins} label="团队经济占比" value={`${Math.round(summary.goldShare * 100)}%`}/><Stat icon={Eye} label="蓝 / 红方" value={`${summary.blueSide} / ${summary.redSide}`}/>{summary.activeSession ? <Stat icon={Activity} label="活跃时段" value={`${summary.activeSession.wins} 胜 ${summary.activeSession.games - summary.activeSession.wins} 负`} detail={`${summary.activeSession.games} 场`}/> : null}</div>
  </section>;
}
