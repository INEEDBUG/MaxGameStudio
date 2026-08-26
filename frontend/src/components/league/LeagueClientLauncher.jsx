import { useEffect, useState } from "react";
import { LoaderCircle, Play, Rocket } from "lucide-react";
import { fetchLeagueClientInstallations, launchLeagueClient } from "../../api/leagueLabApi";

export default function LeagueClientLauncher({ connected = false, onError }) {
  const [installations, setInstallations] = useState([]);
  const [busy, setBusy] = useState("");
  const [started, setStarted] = useState("");

  useEffect(() => {
    let cancelled = false;
    fetchLeagueClientInstallations()
      .then((payload) => { if (!cancelled) setInstallations(payload?.installations || []); })
      .catch(() => { if (!cancelled) setInstallations([]); });
    return () => { cancelled = true; };
  }, [connected]);

  if (!installations.length) return null;

  const launch = async (item) => {
    setBusy(item.kind);
    setStarted("");
    try {
      const result = await launchLeagueClient(item.kind);
      setStarted(result?.label || item.label);
    } catch (error) {
      onError?.(error?.response?.data?.detail || `${item.label} 启动失败`);
    } finally {
      setBusy("");
    }
  };

  return <section className="rounded-2xl border border-violet-400/20 bg-cs2-bg-elevated p-4">
    <div className="flex items-start gap-3">
      <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-violet-400/15 text-violet-200"><Rocket className="h-4 w-4" /></span>
      <div className="min-w-0 flex-1">
        <h2 className="text-sm font-bold">启动英雄联盟</h2>
        <p className="mt-1 text-xs leading-5 text-cs2-text-muted">已按 LeagueAkari 的路径规则检测本机 TCLS、WeGame 与 Riot Client；启动动作仅在点击后执行。</p>
      </div>
      {started && <span className="rounded-full border border-emerald-400/25 bg-emerald-400/10 px-2.5 py-1 text-[11px] text-emerald-200">已启动 {started}</span>}
    </div>
    <div className="mt-3 grid gap-2 md:grid-cols-3">
      {installations.map((item) => <button
        type="button"
        key={item.kind}
        onClick={() => launch(item)}
        disabled={Boolean(busy)}
        title={item.path}
        className="flex min-w-0 items-center gap-3 rounded-xl border border-cs2-border-subtle bg-cs2-bg-input/50 px-3 py-2.5 text-left transition-colors hover:border-violet-300/40 disabled:opacity-50"
      >
        {busy === item.kind ? <LoaderCircle className="h-4 w-4 shrink-0 animate-spin text-violet-200" /> : <Play className="h-4 w-4 shrink-0 text-violet-200" />}
        <span className="min-w-0"><b className="block text-sm">{item.label}</b><span className="mt-0.5 block truncate text-[11px] text-cs2-text-muted">{item.path}</span></span>
      </button>)}
    </div>
  </section>;
}
