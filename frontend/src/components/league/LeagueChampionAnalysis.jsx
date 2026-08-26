import { useMemo, useState } from "react";
import { Crosshair, Eye, Shield, Swords } from "lucide-react";
import { getLeagueChampionIconUrl } from "../../api/api";

const POSITION_LABELS = { TOP: "上路", JUNGLE: "打野", MIDDLE: "中路", BOTTOM: "下路", UTILITY: "辅助" };
const POSITION_COLORS = { TOP: "#e06b6b", JUNGLE: "#55a879", MIDDLE: "#d7a84d", BOTTOM: "#5d8ed6", UTILITY: "#9b79c6" };

function number(value) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function average(rows, getter) {
  return rows.length ? rows.reduce((sum, row) => sum + number(getter(row)), 0) / rows.length : 0;
}

function teamTotal(match, key) {
  const team = (match.participants || []).filter((row) => String(row.team_id) === String(match.team_id));
  return team.reduce((sum, row) => sum + number(row[key]), 0);
}

// Ported from LeagueAkari's MIT-licensed player scoring model.
const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
const linearScore = (value, min, max, cap) => (clamp(value, min, max) - min) / (max - min) * cap;
const expectedScore = (ratio, fullRatio, cap) => linearScore(ratio, 1, fullRatio, cap);

function singleAkariParts(match) {
  const team = (match.participants || []).filter((row) => String(row.team_id) === String(match.team_id));
  const count = team.length;
  const total = (key) => team.reduce((sum, row) => sum + number(row[key]), 0);
  const expectedRatio = (value, teamValue) => count > 1 && teamValue > 0 ? value / teamValue / (1 / count) : 0;
  const teamDamage = total("damage");
  const teamTaken = total("damage_taken");
  const teamGold = total("gold");
  const teamVision = total("vision_score");
  const teamKills = total("kills");
  const durationMinutes = Math.max(1, number(match.duration_seconds) / 60);
  const healingRatio = number(match.healing) / Math.max(1, teamTaken / Math.max(1, count));
  return {
    damage: expectedScore(expectedRatio(number(match.damage), teamDamage), 2, 3),
    damageTaken: expectedScore(expectedRatio(number(match.damage_taken), teamTaken), 2, 2),
    healing: linearScore(healingRatio, .2, count === 1 ? 1 : 1.4, 2),
    cs: linearScore(number(match.cs) / durationMinutes, 5, 10, 2),
    gold: expectedScore(expectedRatio(number(match.gold), teamGold), 1.5, 2),
    participation: linearScore((number(match.kills) + number(match.assists)) / Math.max(1, teamKills), .3, 1, 2),
    vision: expectedScore(expectedRatio(number(match.vision_score), teamVision), 2, 2),
  };
}

export function computeAkariScore(matches = []) {
  if (!matches.length) return null;
  const kills = matches.reduce((sum, row) => sum + number(row.kills), 0);
  const deaths = matches.reduce((sum, row) => sum + number(row.deaths), 0);
  const assists = matches.reduce((sum, row) => sum + number(row.assists), 0);
  const kda = (kills + assists) / Math.max(1, deaths);
  const winRate = matches.filter((row) => row.win).length / matches.length;
  const singles = matches.map(singleAkariParts);
  const mean = (key) => average(singles, (row) => row[key]);
  const parts = {
    kda: clamp(Math.sqrt(Math.max(kda - 2, 0)) * (3 / 7), 0, 1),
    winRate: linearScore(winRate, .5, 1, 1),
    damage: mean("damage"),
    damageTaken: mean("damageTaken"),
    healing: mean("healing"),
    cs: mean("cs"),
    gold: mean("gold"),
    participation: mean("participation"),
    vision: mean("vision"),
  };
  const total = Object.values(parts).reduce((sum, value) => sum + value, 0);
  return { total, maxScore: 17, parts, outstanding: matches.length >= 5 && total >= 6.5, extraordinary: matches.length >= 8 && total >= 8 };
}

export function aggregateChampionMatches(matches = []) {
  const groups = new Map();
  for (const match of matches) {
    const championId = number(match.champion_id);
    if (!championId) continue;
    const group = groups.get(championId) || { championId, championName: match.champion_name || String(championId), rows: [] };
    group.rows.push(match);
    groups.set(championId, group);
  }
  return [...groups.values()].map((group) => {
    const { rows } = group;
    const positions = rows.reduce((result, row) => {
      const position = String(row.position || "").toUpperCase();
      if (POSITION_LABELS[position]) result[position] = (result[position] || 0) + 1;
      return result;
    }, {});
    const kills = rows.reduce((sum, row) => sum + number(row.kills), 0);
    const deaths = rows.reduce((sum, row) => sum + number(row.deaths), 0);
    const assists = rows.reduce((sum, row) => sum + number(row.assists), 0);
    const teamShare = (key) => average(rows, (row) => {
      const total = teamTotal(row, key);
      return total > 0 ? number(row[key]) / total : 0;
    });
    return {
      ...group,
      games: rows.length,
      wins: rows.filter((row) => row.win).length,
      winRate: rows.filter((row) => row.win).length / rows.length,
      kda: (kills + assists) / Math.max(1, deaths),
      averageLine: `${(kills / rows.length).toFixed(1)} / ${(deaths / rows.length).toFixed(1)} / ${(assists / rows.length).toFixed(1)}`,
      damagePerMinute: average(rows, (row) => number(row.damage) / Math.max(1, number(row.duration_seconds) / 60)),
      csPerMinute: average(rows, (row) => number(row.cs) / Math.max(1, number(row.duration_seconds) / 60)),
      visionScore: average(rows, (row) => row.vision_score),
      damageShare: teamShare("damage"),
      damageTakenShare: teamShare("damage_taken"),
      goldShare: teamShare("gold"),
      positions,
      akariScore: computeAkariScore(rows),
    };
  }).sort((a, b) => b.games - a.games || b.winRate - a.winRate);
}

function Metric({ label, value, detail }) {
  return <span className="rounded-xl border border-cs2-border-subtle bg-white/[.025] p-3 text-xs text-cs2-text-muted"><b className="block text-base text-cs2-text-primary">{value}</b>{label}{detail ? <small className="mt-1 block text-[10px]">{detail}</small> : null}</span>;
}

function PositionChart({ positions }) {
  const entries = Object.entries(positions).sort((a, b) => b[1] - a[1]);
  const total = entries.reduce((sum, [, count]) => sum + count, 0);
  let cursor = 0;
  const gradient = entries.map(([position, count]) => {
    const start = cursor;
    cursor += count / Math.max(1, total) * 100;
    return `${POSITION_COLORS[position]} ${start}% ${cursor}%`;
  }).join(", ");
  if (!entries.length) return <p className="text-xs text-cs2-text-muted">这批对局没有可靠的分路数据。</p>;
  return <div className="flex flex-wrap items-center gap-5"><div aria-label="分路分布" className="h-28 w-28 rounded-full" style={{ background: `radial-gradient(circle, var(--color-cs2-bg-elevated, #17191d) 0 43%, transparent 44%), conic-gradient(${gradient})` }}/><div className="min-w-44 flex-1 space-y-1.5">{entries.map(([position, count]) => <div key={position} className="grid grid-cols-[12px_1fr_auto] items-center gap-2 rounded-lg bg-white/[.025] px-2 py-1.5 text-xs"><i className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: POSITION_COLORS[position] }}/><span>{POSITION_LABELS[position]}</span><b>{count} · {Math.round(count / total * 100)}%</b></div>)}</div></div>;
}

export default function LeagueChampionAnalysis({ matches = [] }) {
  const analyses = useMemo(() => aggregateChampionMatches(matches), [matches]);
  const [selectedId, setSelectedId] = useState(null);
  const selected = analyses.find((row) => row.championId === selectedId) || analyses[0];
  if (!selected) return null;
  return <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
    <div className="mb-3 flex items-end justify-between gap-3"><div><h3 className="text-sm font-bold">常用英雄分析</h3><p className="mt-1 text-[10px] text-cs2-text-muted">按当前已读取的 {matches.length} 场战绩本地聚合；收集 100 场可提高可信度。</p></div><span className="text-[10px] text-cs2-text-muted">LeagueAkari 等效分析视图</span></div>
    <div className="mb-4 flex gap-2 overflow-x-auto pb-1">{analyses.slice(0, 12).map((row) => <button key={row.championId} type="button" onClick={() => setSelectedId(row.championId)} className={`flex shrink-0 items-center gap-2 rounded-xl border p-2 text-left ${selected.championId === row.championId ? "border-emerald-400/40 bg-emerald-400/10" : "border-cs2-border-subtle hover:bg-white/[.03]"}`}><img src={getLeagueChampionIconUrl(row.championId)} alt="" className="h-9 w-9 rounded-lg object-cover"/><span><b className="block text-xs">{row.championName}</b><small className="text-[10px] text-cs2-text-muted">{row.games} 场 · {Math.round(row.winRate * 100)}%</small></span></button>)}</div>
    <div className="grid gap-4 lg:grid-cols-[1fr_300px]"><div><div className="mb-3 flex items-center gap-3"><img src={getLeagueChampionIconUrl(selected.championId)} alt="" className="h-12 w-12 rounded-xl object-cover"/><div><b className="text-lg">{selected.championName}</b><p className={`text-xs ${selected.winRate >= .53 ? "text-emerald-300" : selected.winRate <= .47 ? "text-rose-300" : "text-cs2-text-muted"}`}>{selected.wins} 胜 {selected.games - selected.wins} 负 · 胜率 {Math.round(selected.winRate * 100)}%</p></div></div><div className="grid grid-cols-2 gap-2 md:grid-cols-4"><Metric label="Akari Score" value={selected.akariScore.total.toFixed(2)} detail={`${selected.akariScore.maxScore} 分制${selected.akariScore.extraordinary ? " · 非凡" : selected.akariScore.outstanding ? " · 出色" : ""}`}/><Metric label="平均 KDA" value={selected.kda.toFixed(2)} detail={selected.averageLine}/><Metric label="每分钟伤害" value={selected.damagePerMinute.toFixed(0)}/><Metric label="每分钟补刀" value={selected.csPerMinute.toFixed(1)}/><Metric label="平均视野分" value={selected.visionScore.toFixed(1)}/><Metric label="团队伤害占比" value={`${Math.round(selected.damageShare * 100)}%`}/><Metric label="团队承伤占比" value={`${Math.round(selected.damageTakenShare * 100)}%`}/><Metric label="团队经济占比" value={`${Math.round(selected.goldShare * 100)}%`}/><Metric label="样本数" value={`${selected.games} 场`} detail={selected.games < 5 ? "样本较少，仅供参考" : "本地战绩样本"}/></div></div><div className="rounded-xl border border-cs2-border-subtle p-3"><h4 className="mb-3 text-xs font-bold"><Crosshair className="mr-1 inline h-3.5 w-3.5"/>分路分布</h4><PositionChart positions={selected.positions}/></div></div>
    <div className="mt-3 flex flex-wrap gap-3 text-[10px] text-cs2-text-muted"><span><Swords className="mr-1 inline h-3 w-3"/>输出与击杀</span><span><Shield className="mr-1 inline h-3 w-3"/>承伤与经济</span><span><Eye className="mr-1 inline h-3 w-3"/>视野与位置</span></div>
  </section>;
}
