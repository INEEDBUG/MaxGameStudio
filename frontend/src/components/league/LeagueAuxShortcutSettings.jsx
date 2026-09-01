import { useEffect, useState } from "react";
import { Keyboard } from "lucide-react";

const fields = [
  ["ongoing_window_shortcut", "按住显示实时对局窗口", "例如 Ctrl+Alt+O"],
  ["cooldown_window_shortcut", "显示 / 隐藏技能计时器", "例如 Ctrl+Alt+T"],
];

export default function LeagueAuxShortcutSettings({ settings, busy, onSettingsUpdate }) {
  const [drafts, setDrafts] = useState({});
  useEffect(() => {
    setDrafts(Object.fromEntries(fields.map(([key]) => [key, settings?.[key] || ""])));
  }, [settings]);

  const save = async (key) => {
    const value = String(drafts[key] || "").trim() || null;
    if (value === (settings?.[key] || null)) return;
    await onSettingsUpdate({ [key]: value });
  };

  return <section className="rounded-2xl border border-cyan-400/20 bg-cs2-bg-elevated p-4">
    <div className="flex items-start gap-3"><Keyboard className="mt-0.5 h-4 w-4 text-cyan-300"/><div><h3 className="text-sm font-bold">League 全局窗口快捷键</h3><p className="mt-1 text-xs text-cs2-text-muted">留空即关闭。实时对局窗口仅在按住快捷键时显示；其余窗口按一次切换显示状态。快捷键不会绕过任何账号写入保护。</p></div></div>
    <div className="mt-3 grid gap-2 md:grid-cols-2">{fields.map(([key,label,placeholder])=><label key={key} className="rounded-xl border border-cs2-border-subtle p-3 text-xs"><span className="font-semibold">{label}</span><input aria-label={label} value={drafts[key]||""} maxLength={80} disabled={busy} onChange={(event)=>setDrafts((current)=>({...current,[key]:event.target.value}))} onBlur={()=>save(key)} placeholder={placeholder} className="mt-2 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 font-mono text-xs disabled:opacity-40"/></label>)}</div>
  </section>;
}
