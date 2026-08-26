import { useEffect, useMemo, useRef, useState } from "react";
import { BookOpen, Check, Circle, RotateCcw, Save, Search, Shield, Snowflake, Sparkles, Swords, Trash2, Zap } from "lucide-react";
import { fetchLeagueChampions, fetchLeagueLoadoutCatalog } from "../../api/leagueLabApi";
import {
  getLeagueChampionIconUrl,
  getLeaguePerkIconUrl,
  getLeaguePerkStyleIconUrl,
  getLeagueSummonerSpellIconUrl,
} from "../../api/api";

// Mirrors LeagueAkari's auto-champ-config page model. The legacy flat
// champion_loadouts array is accepted as a read-only migration fallback.
const MODES = [
  ["ranked", "排位模式", "CLASSIC"],
  ["normal", "普通模式", "CLASSIC"],
  ["aram", "大乱斗", "ARAM"],
  ["urf", "无限火力", "URF"],
  ["nexusblitz", "极限闪击", "NEXUSBLITZ"],
  ["ultbook", "终极魔典", "ULTBOOK"],
];
const MODE_ICONS = { ranked: Shield, normal: Swords, aram: Snowflake, urf: Zap, nexusblitz: Sparkles, ultbook: BookOpen };
const POSITIONS = [
  ["default", "通用"],
  ["top", "上路"],
  ["jungle", "打野"],
  ["middle", "中路"],
  ["bottom", "下路"],
  ["utility", "辅助"],
];

const clone = (value) => (value == null ? value : JSON.parse(JSON.stringify(value)));
const pageKey = (mode, position) => (mode === "ranked" ? `ranked-${position}` : mode);
const normalizeRune = (value) => (value ? {
  primaryStyleId: Number(value.primaryStyleId ?? value.primary_style_id ?? 0),
  subStyleId: Number(value.subStyleId ?? value.sub_style_id ?? 0),
  selectedPerkIds: (value.selectedPerkIds ?? value.selected_perk_ids ?? []).map(Number),
} : null);
const normalizeSpells = (value) => (value ? {
  spell1Id: Number(value.spell1Id ?? value.spell1_id ?? 0),
  spell2Id: Number(value.spell2Id ?? value.spell2_id ?? 0),
} : null);

function StyleButton({ style, selected, onClick }) {
  return (
    <button type="button" aria-label={`符文系 ${style.name}`} title={style.name} onClick={onClick}
      className={`relative grid h-9 w-9 place-items-center rounded-full border text-[10px] font-bold transition ${selected ? "border-emerald-300 bg-emerald-400/20 text-emerald-200" : "border-cs2-border bg-cs2-bg-input text-cs2-text-muted hover:border-emerald-300/50"}`}>
      {style.icon_path || style.iconPath ? <><span className="text-[9px]">{String(style.name || style.id).slice(0, 2)}</span><img src={getLeaguePerkStyleIconUrl(style.id)} alt="" className="absolute inset-0 h-full w-full object-contain" onError={(event) => { event.currentTarget.style.display = "none"; }} /></> : String(style.name || style.id).slice(0, 2)}
    </button>
  );
}

function PerkButton({ perk, selected, size = "normal", onClick }) {
  return (
    <button type="button" aria-label={perk?.name || String(perk?.id || "符文")} title={perk?.name || String(perk?.id || "")} onClick={onClick}
      className={`relative overflow-hidden rounded-full border transition ${size === "key" ? "h-11 w-11" : size === "stat" ? "h-7 w-7" : "h-9 w-9"} ${selected ? "border-emerald-300 bg-emerald-400/20 ring-2 ring-emerald-300/35" : "border-cs2-border bg-cs2-bg-input hover:border-emerald-300/60"}`}>
      {perk?.id ? <img src={getLeaguePerkIconUrl(perk.id)} alt="" className="h-full w-full object-cover" /> : <span className="grid h-full w-full place-items-center text-[9px] text-cs2-text-muted">?</span>}
    </button>
  );
}

function runeSlots(style) {
  return (style?.slots || []).map((slot, index) => ({ ...slot, slotId: `${style.id}-${slot.type || "slot"}-${index}` }));
}

function RuneEditor({ value, styles, onChange }) {
  const page = value || { primaryStyleId: Number(styles[0]?.id || 0), subStyleId: Number(styles[1]?.id || 0), selectedPerkIds: Array(9).fill(0) };
  const primary = styles.find((style) => Number(style.id) === Number(page.primaryStyleId));
  const secondary = styles.find((style) => Number(style.id) === Number(page.subStyleId));
  const primarySlots = runeSlots(primary);
  const secondarySlots = runeSlots(secondary).filter((slot) => slot.type === "kMixedRegularSplashable" || !slot.type);
  const keySlots = primarySlots.filter((slot) => slot.type === "kKeyStone");
  const regularSlots = primarySlots.filter((slot) => slot.type === "kMixedRegularSplashable" || !slot.type).slice(0, 3);
  const statSlots = primarySlots.filter((slot) => slot.type === "kStatMod").slice(0, 3);
  const selected = page.selectedPerkIds || [];
  const subPerkSlotHistory = useRef(new Map());
  useEffect(() => {
    subPerkSlotHistory.current.clear();
    const slotForPerk = new Map();
    secondarySlots.forEach((slot, index) => (slot.perks || []).forEach((perk) => slotForPerk.set(Number(perk.id), index)));
    selected.slice(4, 6).forEach((perkId) => {
      const slotIndex = slotForPerk.get(Number(perkId));
      if (slotIndex != null) subPerkSlotHistory.current.set(slotIndex, Date.now());
    });
    // The dependency is the style id, not the freshly mapped slot array.
    // This mirrors LeagueAkari's history reset when the secondary style changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [Number(page.subStyleId)]);
  const primaryIndex = (slot) => {
    if (slot.type === "kKeyStone") return 0;
    const regular = regularSlots.indexOf(slot);
    if (regular >= 0) return regular + 1;
    const stat = statSlots.indexOf(slot);
    return stat >= 0 ? stat + 6 : -1;
  };
  const selectPrimaryStyle = (id) => {
    const next = styles.find((style) => Number(style.id) === Number(id));
    const allowed = (next?.allowed_sub_styles || next?.allowedSubStyles || []).map(Number);
    const sub = allowed.includes(Number(page.subStyleId)) ? Number(page.subStyleId) : Number(allowed[0] || styles.find((style) => Number(style.id) !== Number(id))?.id || 0);
    onChange({ primaryStyleId: Number(id), subStyleId: sub, selectedPerkIds: selected.length === 9 ? [...selected] : Array(9).fill(0) });
  };
  const selectPrimaryPerk = (slot, id) => {
    const index = primaryIndex(slot);
    if (index < 0) return;
    const next = selected.length === 9 ? [...selected] : Array(9).fill(0);
    next[index] = Number(id);
    onChange({ ...page, selectedPerkIds: next });
  };
  const selectSubPerk = (slot, id) => {
    const slotForPerk = new Map();
    secondarySlots.forEach((item, index) => (item.perks || []).forEach((perk) => slotForPerk.set(Number(perk.id), index)));
    const next = selected.length === 9 ? [...selected] : Array(9).fill(0);
    const clicked = secondarySlots.indexOf(slot);
    const slotSelections = new Map();
    [...next.slice(4, 6), Number(id)].forEach((perkId) => {
      const slotId = slotForPerk.get(Number(perkId));
      if (slotId != null) slotSelections.set(slotId, Number(perkId));
    });
    if (slotSelections.size > 2) {
      const oldest = [...subPerkSlotHistory.current.entries()].sort((a, b) => a[1] - b[1])[0]?.[0];
      slotSelections.delete(oldest ?? [...slotSelections.keys()][0]);
    }
    subPerkSlotHistory.current.set(clicked, Date.now());
    const ordered = [...slotSelections.entries()].sort((a, b) => a[0] - b[0]).map(([, perkId]) => perkId).slice(0, 2);
    next.splice(4, 2, ...ordered, ...Array(Math.max(0, 2 - ordered.length)).fill(0));
    onChange({ ...page, selectedPerkIds: next });
  };
  return (
    <div className="grid gap-4 rounded-xl border border-cs2-border-subtle bg-black/10 p-4 md:grid-cols-2">
      <div className="space-y-3">
        <div className="flex flex-wrap gap-2">{styles.map((style) => <StyleButton key={style.id} style={style} selected={Number(style.id) === Number(page.primaryStyleId)} onClick={() => selectPrimaryStyle(style.id)} />)}</div>
        {primary ? <>
          <div className="flex flex-wrap justify-center gap-2">{(keySlots[0]?.perks || []).map((perk) => <PerkButton key={perk.id} perk={perk} size="key" selected={Number(selected[0]) === Number(perk.id)} onClick={() => selectPrimaryPerk(keySlots[0], perk.id)} />)}</div>
          {regularSlots.map((slot) => <div key={slot.slotId} className="flex flex-wrap justify-center gap-2">{(slot.perks || []).map((perk) => <PerkButton key={perk.id} perk={perk} selected={Number(selected[primaryIndex(slot)]) === Number(perk.id)} onClick={() => selectPrimaryPerk(slot, perk.id)} />)}</div>)}
        </> : <p className="text-xs text-cs2-text-muted">客户端尚未返回符文系数据。</p>}
      </div>
      <div className="space-y-3 border-t border-cs2-border-subtle pt-3 md:border-l md:border-t-0 md:pl-4 md:pt-0">
        <div className="flex flex-wrap gap-2">{(primary?.allowed_sub_styles || primary?.allowedSubStyles || []).map((id) => { const style = styles.find((item) => Number(item.id) === Number(id)); return style ? <StyleButton key={style.id} style={style} selected={Number(style.id) === Number(page.subStyleId)} onClick={() => onChange({ ...page, subStyleId: Number(style.id), selectedPerkIds: selected.length === 9 ? [...selected] : Array(9).fill(0) })} /> : null; })}</div>
        {secondarySlots.map((slot) => <div key={slot.slotId} className="flex flex-wrap justify-center gap-2">{(slot.perks || []).map((perk) => <PerkButton key={perk.id} perk={perk} selected={selected.slice(4, 6).map(Number).includes(Number(perk.id))} onClick={() => selectSubPerk(slot, perk.id)} />)}</div>)}
        {statSlots.map((slot) => <div key={slot.slotId} className="flex flex-wrap justify-center gap-3">{(slot.perks || []).map((perk) => <PerkButton key={perk.id} perk={perk} size="stat" selected={Number(selected[primaryIndex(slot)]) === Number(perk.id)} onClick={() => selectPrimaryPerk(slot, perk.id)} />)}</div>)}
      </div>
    </div>
  );
}

function validRunes(value, styles) {
  if (!value || Number(value.primaryStyleId) <= 0 || Number(value.subStyleId) <= 0 || (value.selectedPerkIds || []).length !== 9) return false;
  const primary = styles.find((style) => Number(style.id) === Number(value.primaryStyleId));
  const secondary = styles.find((style) => Number(style.id) === Number(value.subStyleId));
  if (!primary || !secondary) return false;
  const selected = value.selectedPerkIds.map(Number);
  const primarySlots = (primary.slots || []).filter((slot) => ["kKeyStone", "kMixedRegularSplashable", "kStatMod"].includes(slot.type));
  if (primarySlots.slice(0, 7).some((slot, index) => !selected[index] || !(slot.perks || []).some((perk) => Number(perk.id) === selected[index]))) return false;
  const secondarySlots = (secondary.slots || []).filter((slot) => slot.type === "kMixedRegularSplashable" || !slot.type);
  const sub = selected.slice(4, 6).filter(Boolean);
  const distinct = new Set(sub.map((id) => secondarySlots.findIndex((slot) => (slot.perks || []).some((perk) => Number(perk.id) === id))));
  return sub.length === 2 && distinct.size === 2;
}

export default function LeagueChampionLoadoutEditor({ settings = {}, onUpdate, onError }) {
  const [catalog, setCatalog] = useState({ champions: [], styles: [], spells: [] });
  const [selectedId, setSelectedId] = useState(null);
  const [mode, setMode] = useState("ranked");
  const [position, setPosition] = useState("default");
  const [kind, setKind] = useState("runes");
  const [query, setQuery] = useState("");
  const [draftRune, setDraftRune] = useState(null);
  const [draftSpells, setDraftSpells] = useState(null);
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(true);
  const runes = settings.runes_v2 || settings.runesV2 || {};
  const spells = settings.summoner_spells || settings.summonerSpells || {};
  const legacy = settings.champion_loadouts || [];
  const key = pageKey(mode, position);
  const selectedKey = selectedId == null ? "" : String(selectedId);

  useEffect(() => {
    let alive = true;
    Promise.all([fetchLeagueChampions(), fetchLeagueLoadoutCatalog()]).then(([champions, loadout]) => {
      if (!alive) return;
      setCatalog({ champions: champions.champions || [], styles: loadout.styles || [], spells: loadout.spells || [] });
      setLoading(false);
    }).catch((error) => {
      if (!alive) return;
      setLoading(false);
      onError?.(error?.response?.data?.detail || "无法读取英雄、符文与召唤师技能目录");
    });
    return () => { alive = false; };
  }, [onError]);

  const configured = (id) => {
    const championRunes = runes[String(id)] || runes[id] || {};
    const championSpells = spells[String(id)] || spells[id] || {};
    const old = legacy.filter((item) => Number(item.champion_id) === Number(id));
    return { runes: Object.values(championRunes).some(Boolean) || old.length > 0, spells: Object.values(championSpells).some(Boolean) || old.length > 0 };
  };
  const championOptions = useMemo(() => {
    const search = query.trim().toLowerCase();
    return [...catalog.champions]
      .filter((champion) => !search || String(champion.name || "").toLowerCase().includes(search) || String(champion.alias || "").toLowerCase().includes(search) || String(champion.id).includes(search))
      .sort((a, b) => {
        const ac = configured(a.id); const bc = configured(b.id);
        return (Number(bc.runes) + Number(bc.spells)) - (Number(ac.runes) + Number(ac.spells)) || String(a.name).localeCompare(String(b.name), "zh-CN");
      });
  }, [catalog.champions, query, runes, spells, legacy]);
  const selectedChampion = catalog.champions.find((item) => Number(item.id) === Number(selectedId));
  const legacyLoadout = useMemo(() => legacy.find((item) => Number(item.champion_id) === Number(selectedId) && String(item.config_key || "default") === key), [legacy, selectedId, key]);
  // Keep these snapshots referentially stable. The page polls League status in
  // the parent; rebuilding a normalized object on every render would make the
  // effect below discard an in-progress edit whenever that poll completes.
  const sourceRune = useMemo(
    () => normalizeRune(runes[selectedKey]?.[key]) || normalizeRune(legacyLoadout),
    [runes, selectedKey, key, legacyLoadout],
  );
  const sourceSpells = useMemo(
    () => normalizeSpells(spells[selectedKey]?.[key]) || normalizeSpells(legacyLoadout),
    [spells, selectedKey, key, legacyLoadout],
  );
  const availableSpells = useMemo(() => {
    const [, , gameMode] = MODES.find(([id]) => id === mode) || MODES[0];
    return catalog.spells.filter((spell) => !spell.game_modes?.length || spell.game_modes.map((value) => String(value).toUpperCase()).includes(gameMode));
  }, [catalog.spells, mode]);
  const currentRune = dirty ? draftRune : sourceRune;
  const currentSpells = dirty ? draftSpells : sourceSpells;
  const isSpellAvailable = (value) => availableSpells.some((spell) => Number(spell.id) === Number(value));

  useEffect(() => { if (selectedId == null && championOptions[0]) setSelectedId(championOptions[0].id); }, [championOptions, selectedId]);
  useEffect(() => { setDirty(false); setDraftRune(clone(sourceRune)); setDraftSpells(clone(sourceSpells)); }, [selectedId, mode, position, kind, sourceRune, sourceSpells]);
  useEffect(() => {
    // LeagueAkari repairs a page when a mode removes a spell from the
    // available list. Keep the repair in the draft so it remains reversible
    // until the user explicitly saves it.
    if (!sourceSpells || availableSpells.length < 2) return;
    const first = availableSpells.find((spell) => Number(spell.id) === Number(sourceSpells.spell1Id)) || availableSpells[0];
    const second = availableSpells.find((spell) => Number(spell.id) === Number(sourceSpells.spell2Id) && Number(spell.id) !== Number(first?.id)) || availableSpells.find((spell) => Number(spell.id) !== Number(first?.id));
    const next = { spell1Id: Number(first?.id || 0), spell2Id: Number(second?.id || 0) };
    if (next.spell1Id !== Number(sourceSpells.spell1Id) || next.spell2Id !== Number(sourceSpells.spell2Id)) {
      setDirty(true);
      setDraftSpells(next);
    }
  }, [sourceSpells, availableSpells]);

  const saveRune = () => {
    if (!selectedId || !validRunes(currentRune, catalog.styles)) return;
    const next = clone(runes);
    next[String(selectedId)] = { ...(next[String(selectedId)] || {}), [key]: { primaryStyleId: Number(currentRune.primaryStyleId), subStyleId: Number(currentRune.subStyleId), selectedPerkIds: currentRune.selectedPerkIds.map(Number) } };
    onUpdate?.({ runes_v2: next });
    setDirty(false);
  };
  const saveSpells = () => {
    if (!selectedId || !currentSpells?.spell1Id || !currentSpells?.spell2Id || Number(currentSpells.spell1Id) === Number(currentSpells.spell2Id) || !isSpellAvailable(currentSpells.spell1Id) || !isSpellAvailable(currentSpells.spell2Id)) return;
    const next = clone(spells);
    next[String(selectedId)] = { ...(next[String(selectedId)] || {}), [key]: { spell1Id: Number(currentSpells.spell1Id), spell2Id: Number(currentSpells.spell2Id) } };
    onUpdate?.({ summoner_spells: next });
    setDirty(false);
  };
  const clearCurrent = () => {
    if (!selectedId) return;
    const next = clone(kind === "runes" ? runes : spells);
    next[String(selectedId)] = { ...(next[String(selectedId)] || {}), [key]: null };
    onUpdate?.(kind === "runes" ? { runes_v2: next } : { summoner_spells: next });
    setDirty(false);
    if (kind === "runes") setDraftRune(null); else setDraftSpells(null);
  };
  const restore = () => { setDirty(false); setDraftRune(clone(sourceRune)); setDraftSpells(clone(sourceSpells)); };
  const createRune = () => {
    const primary = catalog.styles[0];
    const allowed = (primary?.allowed_sub_styles || primary?.allowedSubStyles || []).map(Number);
    const fallback = catalog.styles.find((style) => Number(style.id) !== Number(primary?.id));
    setDirty(true);
    setDraftRune({
      primaryStyleId: Number(primary?.id || 0),
      subStyleId: Number(allowed[0] || fallback?.id || 0),
      selectedPerkIds: Array(9).fill(0),
    });
  };
  const createSpells = () => { setDirty(true); setDraftSpells({ spell1Id: Number(availableSpells[0]?.id || 0), spell2Id: Number(availableSpells.find((spell) => Number(spell.id) !== Number(availableSpells[0]?.id))?.id || 0) }); };

  return (
    <div className="mt-4 rounded-2xl border border-cs2-border bg-cs2-bg-page/30 p-4 text-xs">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div><h3 className="text-sm font-bold text-cs2-text-primary">英雄配置</h3><p className="mt-1 text-cs2-text-muted">按 LeagueAkari 的英雄、模式、分路独立保存符文与召唤师技能。</p></div>
        <span className="rounded-full border border-amber-300/25 bg-amber-300/10 px-2 py-1 text-[10px] text-amber-200">账号写入默认关闭</span>
      </div>
      <div className="grid min-h-[560px] gap-4 lg:grid-cols-[220px_1fr]">
        <aside className="flex min-h-0 flex-col rounded-xl border border-cs2-border bg-cs2-bg-elevated p-2">
          <label className="mb-2 flex items-center gap-2 rounded-lg border border-cs2-border bg-cs2-bg-input px-2"><Search className="h-4 w-4 text-cs2-text-muted"/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索英雄" className="w-full bg-transparent py-2 outline-none"/></label>
          <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto pr-1">
            {loading ? <p className="p-3 text-cs2-text-muted">正在读取英雄目录…</p> : championOptions.map((champion) => { const state = configured(champion.id); return <button type="button" key={champion.id} onClick={() => setSelectedId(champion.id)} className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition ${Number(selectedId) === Number(champion.id) ? "bg-emerald-400/15 text-emerald-200" : "text-cs2-text-secondary hover:bg-white/5"}`}><img src={getLeagueChampionIconUrl(champion.id)} alt="" className="h-7 w-7 rounded-md"/><span className="min-w-0 flex-1 truncate">{champion.name}</span><span className="flex gap-0.5">{state.runes ? <Check className="h-3.5 w-3.5 text-emerald-300" title="已配置符文"/> : <Circle className="h-3 w-3 text-cs2-text-muted" title="未配置符文"/>}{state.spells ? <Check className="h-3.5 w-3.5 text-emerald-300" title="已配置技能"/> : <Circle className="h-3 w-3 text-cs2-text-muted" title="未配置技能"/>}</span></button>; })}
          </div>
        </aside>
        <section className="min-w-0 rounded-xl border border-cs2-border bg-cs2-bg-elevated p-4">
          {selectedChampion ? <>
            <div className="mb-3 flex items-center gap-2"><img src={getLeagueChampionIconUrl(selectedChampion.id)} alt="" className="h-7 w-7 rounded-md"/><h4 className="font-semibold text-cs2-text-primary">{selectedChampion.name}</h4><span className="text-cs2-text-muted">#{selectedChampion.id}</span></div>
            <div className="mb-3 flex flex-wrap gap-1">{MODES.map(([id, label]) => { const ModeIcon = MODE_ICONS[id] || Circle; const modeKey = pageKey(id, position); const modeConfigured = Boolean((runes[selectedKey]?.[modeKey]) || (spells[selectedKey]?.[modeKey]) || (id === "ranked" && legacy.some((item) => Number(item.champion_id) === Number(selectedId) && String(item.config_key || "default").startsWith("ranked-"))) || (id !== "ranked" && legacy.some((item) => Number(item.champion_id) === Number(selectedId) && String(item.config_key || "default") === id))); return <button type="button" key={id} onClick={() => setMode(id)} className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 ${mode === id ? "border-emerald-300/50 bg-emerald-400/15 text-emerald-200" : "border-cs2-border text-cs2-text-muted"}`}><ModeIcon className="h-3.5 w-3.5" aria-hidden="true"/>{label}{modeConfigured ? <Check className="h-3 w-3" aria-label="该模式已有配置"/> : null}</button>; })}</div>
            {mode === "ranked" && <div className="mb-3 flex flex-wrap gap-1">{POSITIONS.map(([id, label]) => <button type="button" key={id} onClick={() => setPosition(id)} className={`rounded-lg border px-2.5 py-1.5 ${position === id ? "border-sky-300/50 bg-sky-400/15 text-sky-200" : "border-cs2-border text-cs2-text-muted"}`}>{label}</button>)}</div>}
            <div className="mb-3 flex gap-1 border-b border-cs2-border-subtle pb-2"><button type="button" onClick={() => setKind("runes")} className={`rounded-lg px-3 py-1.5 font-semibold ${kind === "runes" ? "bg-emerald-400/15 text-emerald-200" : "text-cs2-text-muted"}`}>符文 {currentRune ? <Check className="ml-1 inline h-3 w-3"/> : null}</button><button type="button" onClick={() => setKind("spells")} className={`rounded-lg px-3 py-1.5 font-semibold ${kind === "spells" ? "bg-emerald-400/15 text-emerald-200" : "text-cs2-text-muted"}`}>召唤师技能 {currentSpells ? <Check className="ml-1 inline h-3 w-3"/> : null}</button></div>
            {kind === "runes" ? <>
              {currentRune ? <RuneEditor value={currentRune} styles={catalog.styles} onChange={(next) => { setDirty(true); setDraftRune(clone(next)); }} /> : <div className="grid min-h-[300px] place-items-center rounded-xl border border-dashed border-cs2-border text-cs2-text-muted"><div className="text-center"><p>该场景尚未配置符文</p><button type="button" onClick={createRune} className="mt-3 rounded-lg bg-emerald-400 px-3 py-2 font-semibold text-black">配置符文</button></div></div>}
              <div className="mt-3 flex justify-end gap-2"><button type="button" disabled={!currentRune} onClick={clearCurrent} className="inline-flex items-center gap-1 rounded-lg border border-cs2-border px-3 py-2 text-cs2-text-secondary disabled:opacity-40"><Trash2 className="h-3.5 w-3.5"/>清空</button><button type="button" disabled={!dirty} onClick={restore} className="inline-flex items-center gap-1 rounded-lg border border-cs2-border px-3 py-2 text-cs2-text-secondary disabled:opacity-40"><RotateCcw className="h-3.5 w-3.5"/>恢复</button><button type="button" disabled={!dirty || !validRunes(currentRune, catalog.styles)} onClick={saveRune} className="inline-flex items-center gap-1 rounded-lg bg-emerald-400 px-3 py-2 font-semibold text-black disabled:opacity-40"><Save className="h-3.5 w-3.5"/>保存</button></div>
            </> : <>
              <div className="rounded-xl border border-cs2-border-subtle bg-black/10 p-4"><div className="mb-3 text-cs2-text-muted">召唤师技能（点击图标选择；选择重复技能时自动交换）</div>{currentSpells ? <div className="grid gap-4 md:grid-cols-2">{["spell1Id", "spell2Id"].map((slot, index) => <div key={slot}><div className="mb-2 text-cs2-text-secondary">技能 {index + 1}</div><div className="grid max-h-52 grid-cols-5 gap-2 overflow-y-auto">{availableSpells.map((spell) => <button type="button" key={spell.id} onClick={() => { const next = { ...currentSpells, [slot]: Number(spell.id) }; const other = slot === "spell1Id" ? "spell2Id" : "spell1Id"; if (Number(currentSpells[other]) === Number(spell.id)) next[other] = Number(currentSpells[slot]); setDirty(true); setDraftSpells(next); }} className={`rounded-lg border p-1 ${Number(currentSpells[slot]) === Number(spell.id) ? "border-emerald-300 bg-emerald-400/15" : "border-cs2-border hover:border-emerald-300/50"}`} title={spell.name}><img src={getLeagueSummonerSpellIconUrl(spell.id)} alt={spell.name} className="mx-auto h-8 w-8 rounded"/></button>)}</div></div>)}</div> : <div className="grid min-h-[240px] place-items-center text-cs2-text-muted"><div className="text-center"><p>该场景尚未配置召唤师技能</p><button type="button" onClick={createSpells} className="mt-3 rounded-lg bg-emerald-400 px-3 py-2 font-semibold text-black">配置技能</button></div></div>}</div>
              <div className="mt-3 flex justify-end gap-2"><button type="button" disabled={!currentSpells} onClick={clearCurrent} className="inline-flex items-center gap-1 rounded-lg border border-cs2-border px-3 py-2 text-cs2-text-secondary disabled:opacity-40"><Trash2 className="h-3.5 w-3.5"/>清空</button><button type="button" disabled={!dirty} onClick={restore} className="inline-flex items-center gap-1 rounded-lg border border-cs2-border px-3 py-2 text-cs2-text-secondary disabled:opacity-40"><RotateCcw className="h-3.5 w-3.5"/>恢复</button><button type="button" disabled={!dirty || !currentSpells?.spell1Id || !currentSpells?.spell2Id || Number(currentSpells.spell1Id) === Number(currentSpells.spell2Id) || !isSpellAvailable(currentSpells.spell1Id) || !isSpellAvailable(currentSpells.spell2Id)} onClick={saveSpells} className="inline-flex items-center gap-1 rounded-lg bg-emerald-400 px-3 py-2 font-semibold text-black disabled:opacity-40"><Save className="h-3.5 w-3.5"/>保存</button></div>
            </>}
          </> : <div className="grid min-h-[500px] place-items-center text-cs2-text-muted">从左侧选择英雄开始配置。</div>}
        </section>
      </div>
    </div>
  );
}
