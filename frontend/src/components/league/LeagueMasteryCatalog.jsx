import { useMemo, useState } from "react";
import { RefreshCw, Search, Shield } from "lucide-react";
import { fetchLeaguePlayerMastery } from "../../api/leagueLabApi";
import { getLeagueChampionIconUrl } from "../../api/api";

export default function LeagueMasteryCatalog({ puuid, initialRows = [], onError }) {
  const [rows, setRows] = useState(initialRows);
  const [expanded, setExpanded] = useState(false);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const visible = useMemo(() => {
    const source = expanded ? rows : rows.slice(0, 10);
    const pattern = query.trim().toLowerCase();
    return source.filter((row) => !pattern || `${row.championName || ""} ${row.championId || ""}`.toLowerCase().includes(pattern));
  }, [expanded, query, rows]);

  if (!initialRows.length && !rows.length) return null;

  const loadAll = async () => {
    if (!puuid) return;
    setBusy(true);
    try {
      const payload = await fetchLeaguePlayerMastery(puuid);
      setRows(payload?.mastery || []);
      setExpanded(true);
    } catch (error) {
      onError?.(error?.response?.data?.detail || "完整英雄熟练度读取失败");
    } finally {
      setBusy(false);
    }
  };

  return <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
    <div className="flex flex-wrap items-center gap-2">
      <h3 className="mr-auto text-sm font-bold"><Shield className="mr-1 inline h-4 w-4 text-violet-300"/>英雄熟练度</h3>
      {expanded && <label className="relative"><Search className="absolute left-2.5 top-2 h-3.5 w-3.5 text-cs2-text-muted"/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索英雄" className="w-40 rounded-lg border border-cs2-border bg-cs2-bg-input py-1.5 pl-8 pr-2 text-xs"/></label>}
      <button type="button" onClick={expanded ? () => { setExpanded(false); setQuery(""); } : loadAll} disabled={busy} className="rounded-lg border border-violet-400/25 px-2.5 py-1.5 text-[11px] font-semibold text-violet-200 disabled:opacity-40"><RefreshCw className={`mr-1 inline h-3 w-3 ${busy ? "animate-spin" : ""}`}/>{expanded ? "收起" : "查看全部"}</button>
    </div>
    <div className={`mt-3 grid gap-2 ${expanded ? "max-h-[420px] overflow-y-auto pr-1 sm:grid-cols-2 lg:grid-cols-3" : "sm:grid-cols-2 lg:grid-cols-5"}`}>
      {visible.map((row, index) => <span key={row.championId || index} className="flex items-center gap-2 rounded-lg border border-violet-400/20 bg-violet-400/[.07] px-3 py-2 text-xs">
        <img src={getLeagueChampionIconUrl(row.championId)} alt="" className="h-8 w-8 rounded-lg object-cover"/>
        <span className="min-w-0"><b className="block truncate">{row.championName || `英雄 ${row.championId}`}</b><span>{Number(row.championPoints || 0).toLocaleString()} 点 · {row.championLevel || 0} 级</span></span>
      </span>)}
      {!visible.length && <p className="text-xs text-cs2-text-muted">没有匹配的英雄</p>}
    </div>
  </section>;
}
