import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, History, Trash2 } from "lucide-react";
import { deleteLeaguePlayerEncounter, fetchLeaguePlayerEncounters } from "../../api/leagueLabApi";
import { getLeagueChampionIconUrl } from "../../api/api";
import { formatLeagueTimestamp } from "../../utils/leagueDisplay";

function PlayerLine({ label, player = {} }) {
  return <div className="flex min-w-0 items-center gap-2">
    {player.champion_id ? <img src={getLeagueChampionIconUrl(player.champion_id)} alt="" className="h-8 w-8 rounded-lg object-cover"/> : <span className="h-8 w-8 rounded-lg bg-white/5"/>}
    <span className="min-w-0 text-xs"><b className="block">{label} · {player.champion_name || `英雄 ${player.champion_id || "—"}`}</b><span className="font-mono text-cs2-text-muted">{player.kills ?? "—"}/{player.deaths ?? "—"}/{player.assists ?? "—"}</span></span>
  </div>;
}

export default function LeagueEncounteredGames({ puuid, selfPuuid, onError, emptyLabel = "" }) {
  const [payload, setPayload] = useState(null);
  const [page, setPage] = useState(1);
  const [busy, setBusy] = useState(false);
  const requestSequence = useRef(0);
  const inFlight = useRef(null);
  const load = useCallback(async (nextPage = 1) => {
    const key = `${puuid || ""}|${selfPuuid || ""}|${nextPage}`;
    if (inFlight.current?.key === key) return inFlight.current.promise;
    const request = ++requestSequence.current;
    if (!puuid || !selfPuuid || puuid === selfPuuid) { setPayload(null); setBusy(false); return; }
    setBusy(true);
    const promise = (async () => {
      try {
        const next = await fetchLeaguePlayerEncounters(puuid, nextPage, 10);
        if (request !== requestSequence.current) return;
        setPayload(next && typeof next === "object" ? next : null);
        setPage(nextPage);
      } catch (error) {
        if (request === requestSequence.current) onError?.(error?.response?.data?.detail || "共同对局读取失败");
      } finally {
        if (request === requestSequence.current) setBusy(false);
      }
    })();
    inFlight.current = { key, promise };
    try { return await promise; } finally { if (inFlight.current?.key === key) inFlight.current = null; }
  }, [onError, puuid, selfPuuid]);
  useEffect(() => { setPayload(null); setPage(1); void load(1); }, [load]);

  if (!payload?.total) {
    if (busy) return <section data-testid="encounters-loading" className="rounded-2xl border border-cyan-400/20 bg-cs2-bg-elevated p-4 text-xs text-cs2-text-muted">正在读取共同对局…</section>;
    return emptyLabel ? <section data-testid="encounters-empty" className="rounded-2xl border border-dashed border-cs2-border-subtle bg-cs2-bg-elevated p-4 text-xs text-cs2-text-muted">{emptyLabel}</section> : null;
  }
  const total = Number(payload?.total) || 0;
  const pageSize = Math.max(1, Number(payload?.page_size) || 10);
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  return <section className="rounded-2xl border border-cyan-400/20 bg-cs2-bg-elevated p-4">
    <div className="flex items-center gap-2"><History className="h-4 w-4 text-cyan-300"/><h3 className="mr-auto text-sm font-bold">共同对局（{total}）</h3><button disabled={page <= 1 || busy} onClick={() => load(page - 1)} className="rounded-lg border border-cs2-border p-1.5 disabled:opacity-30"><ChevronLeft className="h-3.5 w-3.5"/></button><span className="text-[11px] text-cs2-text-muted">{page}/{totalPages}</span><button disabled={page >= totalPages || busy} onClick={() => load(page + 1)} className="rounded-lg border border-cs2-border p-1.5 disabled:opacity-30"><ChevronRight className="h-3.5 w-3.5"/></button></div>
    <div className="mt-3 grid gap-2 md:grid-cols-2">
      {(Array.isArray(payload.games) ? payload.games : []).map((game, index) => <article key={`${game.self_puuid || selfPuuid}:${game.game_id ?? "unknown"}:${index}`} className={`rounded-xl border p-3 ${game.target?.win === true ? "border-emerald-400/20 bg-emerald-400/[.05]" : game.target?.win === false ? "border-rose-400/20 bg-rose-400/[.05]" : "border-cs2-border-subtle"}`}>
        <div className="mb-2 flex items-center justify-between gap-2 text-[11px] text-cs2-text-muted"><span>{game.game_mode || "未知模式"} · {formatLeagueTimestamp(game.played_at)}</span><button aria-label={`移除共同对局 ${game.game_id ?? "未知"}`} disabled={busy} onClick={async () => { try { await deleteLeaguePlayerEncounter(puuid, game.game_id); await load(page); } catch (error) { onError?.(error?.response?.data?.detail || "移除共同对局失败"); } }} className="rounded p-1 text-rose-300 hover:bg-rose-400/10 disabled:opacity-40"><Trash2 className="h-3.5 w-3.5"/></button></div>
        <div className="grid gap-2 sm:grid-cols-2"><PlayerLine label="该玩家" player={game.target}/><PlayerLine label="我" player={game.self}/></div>
      </article>)}
    </div>
  </section>;
}
