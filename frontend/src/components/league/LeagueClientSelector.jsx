import { useCallback, useEffect, useState } from "react";
import { Check, LoaderCircle, MonitorSmartphone } from "lucide-react";
import { getLeagueProfileIconUrl } from "../../api/api";
import { fetchLeagueClients, selectLeagueClient } from "../../api/leagueLabApi";

function clientLabel(client, streamerMode) {
  if (streamerMode) return `League 客户端 · PID ${client.pid}`;
  const riotId = [client.game_name, client.tag_line].filter(Boolean).join("#");
  return riotId || `League 客户端 · PID ${client.pid}`;
}

export default function LeagueClientSelector({ streamerMode = false, onSelected, onError }) {
  const [clients, setClients] = useState([]);
  const [selectedPid, setSelectedPid] = useState(0);
  const [busyPid, setBusyPid] = useState(0);

  const load = useCallback(async () => {
    try {
      const payload = await fetchLeagueClients();
      setClients(payload?.clients || []);
      setSelectedPid(Number(payload?.selected_pid || 0));
    } catch {
      setClients([]);
    }
  }, []);

  useEffect(() => {
    load();
    const timer = window.setInterval(load, 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  if (clients.length < 2) return null;

  const choose = async (pid) => {
    if (pid === selectedPid || busyPid) return;
    setBusyPid(pid);
    try {
      await selectLeagueClient(pid);
      setSelectedPid(pid);
      await onSelected?.();
      await load();
    } catch (error) {
      onError?.(error?.response?.data?.detail || "切换 League 客户端失败");
    } finally {
      setBusyPid(0);
    }
  };

  return <section className="rounded-2xl border border-cyan-400/25 bg-cyan-400/[.05] p-4">
    <div className="flex items-start gap-3">
      <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-cyan-400/15 text-cyan-200"><MonitorSmartphone className="h-4 w-4" /></span>
      <div className="min-w-0 flex-1">
        <h2 className="text-sm font-bold text-cs2-text-primary">检测到 {clients.length} 个 League 客户端</h2>
        <p className="mt-1 text-xs leading-5 text-cs2-text-muted">选择要由实验室读取和自动化的账号；认证令牌仍只保存在内存中。</p>
      </div>
    </div>
    <div className="mt-3 grid gap-2 md:grid-cols-2">
      {clients.map((client) => {
        const selected = client.pid === selectedPid || client.selected;
        return <button
          type="button"
          key={client.pid}
          onClick={() => choose(client.pid)}
          disabled={Boolean(busyPid)}
          className={`flex items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition-colors disabled:opacity-50 ${selected ? "border-emerald-400/45 bg-emerald-400/10" : "border-cs2-border-subtle bg-cs2-bg-elevated hover:border-cyan-300/40"}`}
        >
          {client.profile_icon_id != null
            ? <img src={getLeagueProfileIconUrl(client.profile_icon_id)} alt="" className="h-9 w-9 rounded-lg object-cover" />
            : <span className="grid h-9 w-9 place-items-center rounded-lg bg-white/5 text-xs font-bold">L</span>}
          <span className="min-w-0 flex-1">
            <b className="block truncate text-sm">{clientLabel(client, streamerMode)}</b>
            <span className="mt-0.5 block truncate text-[11px] text-cs2-text-muted">{client.phase || "Unknown"} · {client.platform_id || client.region || "本地区服"} · PID {client.pid}</span>
          </span>
          {busyPid === client.pid ? <LoaderCircle className="h-4 w-4 animate-spin text-cyan-200" /> : selected ? <Check className="h-4 w-4 text-emerald-300" /> : null}
        </button>;
      })}
    </div>
  </section>;
}
