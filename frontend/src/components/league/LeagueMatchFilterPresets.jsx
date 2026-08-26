import { useMemo, useState } from "react";
import { BookmarkPlus, Trash2 } from "lucide-react";

const STORAGE_KEY = "league-player-filter-presets";

function readPresets() {
  try {
    const value = JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
    return Array.isArray(value) ? value.filter((row) => row?.name && row?.filter) : [];
  } catch {
    return [];
  }
}

function persistPresets(presets) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(presets));
}

export default function LeagueMatchFilterPresets({ filter, onApply }) {
  const [name, setName] = useState("");
  const [presets, setPresets] = useState(readPresets);
  const normalizedName = name.trim();
  const canSave = normalizedName.length > 0 && normalizedName.length <= 24;
  const activePreset = useMemo(
    () => presets.find((preset) => JSON.stringify(preset.filter) === JSON.stringify(filter))?.name || "",
    [filter, presets],
  );

  const save = () => {
    if (!canSave) return;
    const next = [...presets.filter((preset) => preset.name !== normalizedName), { name: normalizedName, filter: { ...filter } }].slice(-12);
    persistPresets(next);
    setPresets(next);
    setName("");
  };

  const remove = (presetName) => {
    const next = presets.filter((preset) => preset.name !== presetName);
    persistPresets(next);
    setPresets(next);
  };

  return (
    <section className="rounded-xl border border-cs2-border bg-cs2-bg-elevated p-3">
      <div className="flex flex-col gap-2 lg:flex-row lg:items-center">
        <div className="flex min-w-0 flex-1 gap-2">
          <input value={name} maxLength={24} onChange={(event) => setName(event.target.value)} onKeyDown={(event) => event.key === "Enter" && save()} placeholder="为当前筛选命名，例如：排位高 KDA" className="min-w-0 flex-1 rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs" />
          <button type="button" disabled={!canSave} onClick={save} className="rounded-lg border border-emerald-400/30 bg-emerald-400/10 px-3 py-2 text-xs font-semibold text-emerald-200 disabled:opacity-40"><BookmarkPlus className="mr-1 inline h-3.5 w-3.5" />保存筛选</button>
        </div>
        <span className="text-[10px] text-cs2-text-muted">仅保存在本机，最多 12 组</span>
      </div>
      {presets.length > 0 && <div className="mt-3 flex flex-wrap gap-2">{presets.map((preset) => <span key={preset.name} className={`inline-flex overflow-hidden rounded-lg border ${activePreset === preset.name ? "border-cyan-400/50 bg-cyan-400/10" : "border-cs2-border-subtle bg-white/[.025]"}`}><button type="button" onClick={() => onApply({ ...preset.filter })} className="px-3 py-1.5 text-xs hover:bg-white/[.05]">{preset.name}</button><button type="button" aria-label={`删除筛选 ${preset.name}`} onClick={() => remove(preset.name)} className="border-l border-current/10 px-2 text-cs2-text-muted hover:bg-rose-400/10 hover:text-rose-300"><Trash2 className="h-3.5 w-3.5" /></button></span>)}</div>}
    </section>
  );
}
