import { useCallback, useEffect, useState } from "react";
import { Download, LoaderCircle, Play } from "lucide-react";
import { downloadLeagueReplay, fetchLeagueReplay, watchLeagueReplay } from "../../api/leagueLabApi";

export default function LeagueMatchReplayActions({ match, onError }) {
  const [replay, setReplay] = useState(null);
  const [busy, setBusy] = useState(false);
  const gameId = Number(match?.game_id || 0);

  const load = useCallback(async () => {
    if (!gameId) return;
    try {
      setReplay(await fetchLeagueReplay(gameId));
    } catch {
      setReplay(null);
    }
  }, [gameId]);

  useEffect(() => { load(); }, [load]);
  const state = replay?.metadata?.state || "";
  useEffect(() => {
    if (!['checking', 'downloading'].includes(state)) return undefined;
    const timer = window.setTimeout(load, 1500);
    return () => window.clearTimeout(timer);
  }, [state, load]);

  if (!replay?.enabled) return null;
  const watching = state === "watch";
  const downloading = state === "downloading" || busy;
  const disabled = !["download", "watch", "downloading"].includes(state) || downloading;
  const title = watching
    ? "播放回放"
    : downloading
      ? `下载中 ${Math.round(Number(replay?.metadata?.downloadProgress || 0))}%`
      : state === "incompatible"
        ? "版本不兼容"
        : "下载回放";

  const run = async () => {
    setBusy(true);
    try {
      if (watching) await watchLeagueReplay(gameId);
      else await downloadLeagueReplay(gameId, match);
      await load();
    } catch (error) {
      onError?.(error?.response?.data?.detail || `${title}失败`);
    } finally {
      setBusy(false);
    }
  };

  return <button
    type="button"
    onClick={run}
    disabled={disabled}
    title={title}
    aria-label={`${title} ${gameId}`}
    className="inline-flex items-center gap-1 rounded-lg border border-cyan-400/20 bg-cyan-400/[.06] px-2 py-1 text-[10px] font-semibold text-cyan-200 disabled:opacity-40"
  >
    {downloading ? <LoaderCircle className="h-3 w-3 animate-spin" /> : watching ? <Play className="h-3 w-3" /> : <Download className="h-3 w-3" />}
    {title}
  </button>;
}
