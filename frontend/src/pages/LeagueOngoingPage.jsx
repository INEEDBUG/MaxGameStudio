import { useCallback, useEffect, useRef, useState } from "react";
import { Gamepad2, MonitorUp, RefreshCw, ShieldCheck } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { fetchLeagueLabStatus, saveLeagueLabSettings } from "../api/leagueLabApi";
import LeagueOngoingGame from "../components/league/LeagueOngoingGame";
import LeagueOngoingSettings from "../components/league/LeagueOngoingSettings";

const DEFAULT_SETTINGS = {
  ongoing_show_streak_tags: true,
  ongoing_show_performance_tags: true,
  ongoing_query_concurrency: 10,
  ongoing_premade_threshold: 5,
  ongoing_match_history_load_count: 20,
  ongoing_jungle_analysis_count: 4,
  ongoing_show_jungle_pathing: true,
  ongoing_show_premade_tag: true,
  ongoing_show_local_tag: true,
  ongoing_auto_route_when_game_starts: false,
  streamer_mode_enabled: false,
  streamer_mode_use_aliases: false,
};

export default function LeagueOngoingPage() {
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const settingsRef = useRef(settings);

  const load = useCallback(async () => {
    setBusy(true);
    try {
      const next = await fetchLeagueLabStatus();
      const merged = { ...DEFAULT_SETTINGS, ...(next?.settings || {}) };
      settingsRef.current = merged;
      setSettings(merged);
      setError("");
    } catch (err) {
      setError(err?.response?.data?.detail || err?.message || "无法读取英雄联盟客户端状态");
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  const update = useCallback(async (patch) => {
    const next = { ...settingsRef.current, ...(typeof patch === "function" ? patch(settingsRef.current) : patch) };
    settingsRef.current = next;
    setSettings(next);
    setBusy(true);
    try {
      const response = await saveLeagueLabSettings(next);
      if (response) {
        const saved = { ...DEFAULT_SETTINGS, ...(response.settings || next) };
        settingsRef.current = saved;
        setSettings(saved);
      }
      setError("");
    } catch (err) {
      const message = err?.response?.data?.detail || err?.message || "设置保存失败";
      await load();
      setError(message);
    } finally {
      setBusy(false);
    }
  }, [load]);

  return (
    <div className="mx-auto h-full w-full max-w-[1220px] space-y-5 overflow-y-auto px-7 py-7">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <div className="mb-2 inline-flex items-center gap-2 rounded-full border border-cyan-400/25 bg-cyan-400/10 px-2.5 py-1 text-[11px] font-bold uppercase tracking-[.16em] text-cyan-300"><Gamepad2 className="h-3.5 w-3.5" /> League Live Game</div>
          <h1 className="text-2xl font-bold tracking-[-.03em] text-cs2-text-primary">实时对局</h1>
          <p className="mt-1 text-sm text-cs2-text-secondary">查看当前队伍、近期战绩、组排关系和实时分析。</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" aria-label="打开独立实时对局窗口" onClick={() => invoke("open_league_ongoing").catch(() => setError("独立实时对局窗口尚未打开"))} className="inline-flex items-center gap-2 rounded-xl border border-cs2-border px-3 py-2 text-xs font-semibold text-cs2-text-secondary"><MonitorUp className="h-4 w-4" />独立窗口</button>
          <button type="button" aria-label="刷新实时对局状态" onClick={load} disabled={busy} className="inline-flex items-center gap-2 rounded-xl border border-cs2-border px-3 py-2 text-xs font-semibold text-cs2-text-secondary disabled:opacity-50"><RefreshCw className={`h-4 w-4 ${busy ? "animate-spin" : ""}`} />刷新</button>
        </div>
      </header>
      {error && <div role="alert" className="rounded-xl border border-red-400/30 bg-red-400/10 px-4 py-3 text-sm text-red-300">{error}</div>}
      <LeagueOngoingSettings settings={settings} onUpdate={update} />
      <LeagueOngoingGame streamerMode={settings.streamer_mode_enabled} useAliases={settings.streamer_mode_use_aliases} onError={setError} />
      <div className="flex items-start gap-3 rounded-2xl border border-cs2-border-subtle bg-cs2-bg-elevated/60 p-4 text-xs leading-5 text-cs2-text-muted"><ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-300" /><p>实时对局只读读取本机 League LCU 数据；详情和路线分析会按需加载，单个玩家数据失败不会阻塞整页。</p></div>
    </div>
  );
}
