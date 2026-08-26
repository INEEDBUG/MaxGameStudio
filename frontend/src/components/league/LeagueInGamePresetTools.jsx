import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, Keyboard, Plus, RefreshCw, Save, Send, Trash2, X } from "lucide-react";
import {
  cancelLeagueInGameSend,
  fetchLeagueLabStatus,
  fetchLeagueOngoingGame,
  sendLeagueInGameLines,
  sendLeagueInGamePreset,
} from "../../api/leagueLabApi";
import {
  buildLeaguePresetLines,
  getLeaguePresetOptions,
  NAME_STRATEGIES,
  playerKey,
  playerName,
  presetOptionsKey,
  RATING_OPTIONS,
  JUNGLE_OPTIONS,
  selectLeaguePresetPlayers,
  serializeLeaguePresetOptions,
  SHORTCUT_TARGETS,
  TARGET_OPTIONS,
  normalizeLeaguePresetOptions,
} from "../../utils/leagueChatPresets";

const createPresetId = () => `preset_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;

function Toggle({ label, description, checked, disabled, onChange }) {
  return <label className={`flex items-center justify-between gap-3 rounded-lg border border-cs2-border-subtle px-3 py-2 text-xs ${disabled ? "opacity-40" : ""}`}>
    <span><b className="block">{label}</b>{description ? <small className="mt-0.5 block text-[10px] text-cs2-text-muted">{description}</small> : null}</span>
    <input type="checkbox" aria-label={label} checked={Boolean(checked)} disabled={disabled} onChange={(event) => onChange(event.target.checked)} className="h-4 w-4 accent-indigo-400"/>
  </label>;
}

function TargetPicker({ kind, draft, players, ownPuuid, disabled, streamerMode, useAliases, onChange }) {
  const own = players.find((player) => String(player?.puuid || player?.playerPuuid || "") === String(ownPuuid || ""));
  return <section className="rounded-xl border border-cs2-border-subtle p-3">
    <div className="flex flex-wrap items-center gap-2"><b className="text-xs">目标玩家</b><select aria-label={`${kind}目标范围`} value={draft.targetMode} disabled={disabled} onChange={(event) => onChange({ targetMode: event.target.value })} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs">{TARGET_OPTIONS.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select><span className="text-[10px] text-cs2-text-muted">{players.length ? `已读取 ${players.length} 人` : "请先读取当前玩家"}</span></div>
    {draft.targetMode === "selected" ? <div className="mt-2 grid max-h-44 gap-1 overflow-y-auto pr-1 sm:grid-cols-2">{players.map((player, index) => { const key = playerKey(player, index); const visibleName = playerName(player, index, draft.nameDisplayStrategy, streamerMode, useAliases); return <label key={key} className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-xs hover:bg-white/5"><input type="checkbox" aria-label={`选择 ${visibleName}`} checked={draft.selectedPuuids.includes(key)} disabled={disabled} onChange={(event) => onChange({ selectedPuuids: event.target.checked ? [...new Set([...draft.selectedPuuids, key])] : draft.selectedPuuids.filter((value) => value !== key) })} className="h-3.5 w-3.5 accent-indigo-400"/><span className="truncate">{visibleName}</span></label>; })}</div> : null}
    {(draft.targetMode === "friendly" || draft.targetMode === "enemy") && !own ? <p className="mt-2 text-[10px] text-amber-200">当前数据中没有匹配本机召唤师，无法证明己方/敌方范围；请改用双方全部或手动选择。</p> : null}
  </section>;
}

function ShortcutGrid({ kind, settings, busy, enabled, onSave }) {
  const key = `in_game_${kind}_shortcuts`;
  return <div className="mt-3 rounded-xl border border-cs2-border-subtle p-3"><b className="text-xs">{kind === "rating" ? "评分" : kind === "jungle" ? "打野画像" : "组排关系"}快捷键</b><p className="mt-1 text-[10px] text-cs2-text-muted">留空即关闭；快捷键触发时读取后端保存的对应选项，并使用与页面预览完全相同的生成器。</p><div className="mt-2 grid gap-2 sm:grid-cols-3">{SHORTCUT_TARGETS.map(([target, label]) => <label key={target} className="text-[10px] text-cs2-text-muted"><span>{label}</span><input aria-label={`${kind === "rating" ? "近期表现" : kind === "jungle" ? "打野画像" : "组排关系"}${label}快捷键`} defaultValue={settings?.[key]?.[target] || ""} maxLength={80} disabled={!enabled || busy} onBlur={(event) => onSave(target, event.target.value)} placeholder="未设置" className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 font-mono text-xs disabled:opacity-40"/></label>)}</div></div>;
}

export default function LeagueInGamePresetTools({ settings, busy, onSettingsUpdate, onBusyChange, onError, streamerMode = false, useAliases = false }) {
  const [tab, setTab] = useState("rating");
  const [items, setItems] = useState(() => Array.isArray(settings?.in_game_fixed_presets) ? settings.in_game_fixed_presets : []);
  const [draftOverrides, setDraftOverrides] = useState({});
  const [players, setPlayers] = useState([]);
  const [ownPuuid, setOwnPuuid] = useState("");
  const [playersBusy, setPlayersBusy] = useState(false);
  const [playersError, setPlayersError] = useState("");
  const [previews, setPreviews] = useState({ rating: [], jungle: [], premade: [] });
  const playersRequest = useRef(0);
  const migrationAttempted = useRef(false);

  useEffect(() => setItems(Array.isArray(settings?.in_game_fixed_presets) ? settings.in_game_fixed_presets : []), [settings?.in_game_fixed_presets]);
  useEffect(() => {
    if (migrationAttempted.current) return;
    // The page starts with a local defaults object before the first backend
    // status arrives. Wait for the status marker so a legacy draft can never
    // race the initial settings load.
    if (!Object.prototype.hasOwnProperty.call(settings || {}, "safety_migration_version")) return;
    const kinds = ["rating", "jungle", "premade"];
    const missing = kinds.filter((kind) => !Object.prototype.hasOwnProperty.call(settings || {}, presetOptionsKey(kind)));
    if (!missing.length) { migrationAttempted.current = true; return; }
    let legacy = null;
    try { legacy = JSON.parse(window.localStorage.getItem("league-in-game-preset-draft-v1") || "null"); } catch { legacy = null; }
    migrationAttempted.current = true;
    if (!legacy || typeof legacy !== "object") return;
    const patch = Object.fromEntries(missing.map((kind) => [presetOptionsKey(kind), serializeLeaguePresetOptions(kind, legacy[kind])]));
    void Promise.resolve(onSettingsUpdate(patch)).then(() => {
      try { window.localStorage.removeItem("league-in-game-preset-draft-v1"); } catch { /* best effort cleanup */ }
    }).catch((error) => onError(error?.response?.data?.detail || "迁移游戏内预设配置失败"));
  }, [settings, onSettingsUpdate, onError]);
  useEffect(() => {
    // Keep a short-lived optimistic view while the parent saves. Once the
    // backend response contains the same canonical value, let settings be
    // the sole source again. No draft is written to localStorage.
    setDraftOverrides((current) => {
      const next = { ...current };
      let changed = false;
      for (const kind of ["rating", "jungle", "premade"]) {
        if (!next[kind]) continue;
        const canonical = serializeLeaguePresetOptions(kind, getLeaguePresetOptions(settings, kind));
        const optimistic = serializeLeaguePresetOptions(kind, next[kind]);
        if (JSON.stringify(canonical) === JSON.stringify(optimistic)) {
          delete next[kind];
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [settings?.in_game_rating_preset_options, settings?.in_game_jungle_preset_options, settings?.in_game_premade_preset_options]);
  useEffect(() => () => { playersRequest.current += 1; }, []);

  const enabled = Boolean(settings?.toolkit_account_actions_enabled && settings?.in_game_send_enabled);
  const drafts = {
    rating: normalizeLeaguePresetOptions("rating", { ...getLeaguePresetOptions(settings, "rating"), ...(draftOverrides.rating || {}), display: { ...getLeaguePresetOptions(settings, "rating").display, ...(draftOverrides.rating?.display || {}) } }),
    jungle: normalizeLeaguePresetOptions("jungle", { ...getLeaguePresetOptions(settings, "jungle"), ...(draftOverrides.jungle || {}), display: { ...getLeaguePresetOptions(settings, "jungle").display, ...(draftOverrides.jungle?.display || {}) } }),
    premade: normalizeLeaguePresetOptions("premade", { ...getLeaguePresetOptions(settings, "premade"), ...(draftOverrides.premade || {}) }),
  };
  const updateDraft = (kind, patch) => {
    const current = drafts[kind];
    const merged = normalizeLeaguePresetOptions(kind, {
      ...current,
      ...patch,
      display: patch.display ? { ...current.display, ...patch.display } : current.display,
    });
    setDraftOverrides((existing) => ({ ...existing, [kind]: merged }));
    try {
      return onSettingsUpdate({ [presetOptionsKey(kind)]: serializeLeaguePresetOptions(kind, merged) });
    } catch (error) {
      setDraftOverrides((existing) => { const next = { ...existing }; delete next[kind]; return next; });
      onError(error?.response?.data?.detail || "保存游戏内预设配置失败");
      return undefined;
    }
  };
  const persist = async (next) => { onBusyChange(true); try { await onSettingsUpdate({ in_game_fixed_presets: next }); setItems(next); } catch (error) { onError(error?.response?.data?.detail || "保存游戏内预设失败"); } finally { onBusyChange(false); } };
  const patchItem = (id, patch) => setItems((current) => current.map((item) => item.id === id ? { ...item, ...patch } : item));
  const add = () => persist([...items, { id: createPresetId(), title: "未命名预设", shortcut: null, content: "" }]);
  const remove = (id) => { if (window.confirm("删除这条固定文字预设？")) persist(items.filter((item) => item.id !== id)); };
  const move = (id, offset) => { const index = items.findIndex((item) => item.id === id); const target = index + offset; if (index < 0 || target < 0 || target >= items.length) return; const next = [...items]; [next[index], next[target]] = [next[target], next[index]]; persist(next); };
  const sendFixed = async (id) => { if (window.prompt("该操作会向当前房间、英雄选择或前台游戏聊天发送预设内容。\n请输入“我确认发送”继续：") !== "我确认发送") return; onBusyChange(true); try { await sendLeagueInGamePreset(id, "manual", "我确认发送"); } catch (error) { onError(error?.response?.data?.detail || "发送预设失败"); } finally { onBusyChange(false); } };
  const toggle = async (value) => { if (value && !window.confirm("启用后，已配置快捷键的固定文字预设可在软件驻留后台时发送；只有房间、英雄选择或前台英雄联盟游戏阶段会执行。确定启用吗？")) return; await onSettingsUpdate({ in_game_send_enabled: value }); };
  const saveGeneratedShortcut = (kind, target, value) => { const key = `in_game_${kind}_shortcuts`; return onSettingsUpdate({ [key]: { friendly: null, enemy: null, all: null, ...(settings?.[key] || {}), [target]: String(value || "").trim() || null } }); };

  const readPlayers = async () => {
    const request = ++playersRequest.current;
    setPlayersBusy(true); setPlayersError("");
    try {
      const [game, status] = await Promise.all([fetchLeagueOngoingGame(), fetchLeagueLabStatus()]);
      if (request !== playersRequest.current) return { players: [], ownPuuid: "" };
      const nextPlayers = Array.isArray(game?.players) ? game.players : [];
      const nextOwn = String(status?.current_summoner?.puuid || "");
      setPlayers(nextPlayers); setOwnPuuid(nextOwn);
      if (!nextPlayers.length) setPlayersError("当前没有可读取的房间或对局玩家。");
      return { players: nextPlayers, ownPuuid: nextOwn };
    } catch (error) {
      if (request !== playersRequest.current) return { players: [], ownPuuid: "" };
      const message = error?.response?.data?.detail || error?.message || "实时玩家读取失败";
      setPlayersError(message); onError(message); return { players: [], ownPuuid: "" };
    } finally { if (request === playersRequest.current) setPlayersBusy(false); }
  };

  const generate = async (kind) => {
    let sourcePlayers = players; let sourceOwn = ownPuuid;
    if (!sourcePlayers.length) ({ players: sourcePlayers, ownPuuid: sourceOwn } = await readPlayers());
    const selected = selectLeaguePresetPlayers(sourcePlayers, drafts[kind], sourceOwn);
    const privacy = { streamerMode, useAliases };
    const lines = buildLeaguePresetLines(kind, selected, drafts[kind], privacy);
    setPreviews((current) => ({ ...current, [kind]: lines }));
  };

  const sendGenerated = async (kind) => {
    const lines = previews[kind] || [];
    if (!lines.length) { onError("请先读取玩家并生成预览"); return; }
    if (window.prompt("该操作会向当前房间、英雄选择或前台游戏聊天发送生成的预设。\n请输入“我确认发送”继续：") !== "我确认发送") return;
    onBusyChange(true);
    try { await sendLeagueInGameLines(lines.slice(0, 10), "我确认发送", "manual", kind, drafts[kind].targetMode === "selected" ? null : drafts[kind].targetMode); } catch (error) { onError(error?.response?.data?.detail || "发送生成预设失败"); } finally { onBusyChange(false); }
  };

  const cancelSend = async () => { onBusyChange(true); try { await cancelLeagueInGameSend(); } catch (error) { onError(error?.response?.data?.detail || "取消发送失败"); } finally { onBusyChange(false); } };
  const renderGenerated = (kind, label, description, options) => {
    const draft = drafts[kind];
    const lines = previews[kind] || [];
    return <div className="space-y-3">
      <p className="text-xs leading-5 text-cs2-text-muted">{description}</p>
      <div className="flex flex-wrap gap-2"><button onClick={readPlayers} disabled={playersBusy || busy} className="inline-flex items-center gap-1 rounded-lg border border-cs2-border px-3 py-1.5 text-xs disabled:opacity-40"><RefreshCw className={`h-3.5 w-3.5 ${playersBusy ? "animate-spin" : ""}`}/>读取当前玩家</button><button onClick={() => generate(kind)} disabled={playersBusy || busy || !players.length} className="rounded-lg border border-indigo-400/30 bg-indigo-400/10 px-3 py-1.5 text-xs font-semibold text-indigo-200 disabled:opacity-40">生成预览</button><button onClick={() => sendGenerated(kind)} disabled={!enabled || busy || !lines.length} className="inline-flex items-center gap-1 rounded-lg border border-emerald-400/25 bg-emerald-400/10 px-3 py-1.5 text-xs font-semibold text-emerald-200 disabled:opacity-40"><Send className="h-3.5 w-3.5"/>发送预览</button><button onClick={cancelSend} disabled={!enabled || busy} className="inline-flex items-center gap-1 rounded-lg border border-rose-400/25 bg-rose-400/10 px-3 py-1.5 text-xs font-semibold text-rose-200 disabled:opacity-40"><X className="h-3.5 w-3.5"/>取消发送</button></div>
      {playersError ? <div role="alert" className="rounded-lg border border-amber-400/25 bg-amber-400/[.05] px-3 py-2 text-xs text-amber-200">{playersError}</div> : null}
      {players.length ? <TargetPicker kind={label} draft={draft} players={players} ownPuuid={ownPuuid} streamerMode={streamerMode} useAliases={useAliases} disabled={busy || playersBusy} onChange={(patch) => updateDraft(kind, patch)}/> : null}
      <div className="grid gap-3 lg:grid-cols-2"><section className="space-y-2 rounded-xl border border-cs2-border-subtle p-3"><b className="text-xs">名字与显示配置</b><label className="mt-2 block text-xs text-cs2-text-muted">名字显示策略<select aria-label={`${label}名字显示策略`} value={draft.nameDisplayStrategy} disabled={busy} onChange={(event) => updateDraft(kind, { nameDisplayStrategy: event.target.value })} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs">{NAME_STRATEGIES.map(([value, text]) => <option key={value} value={value}>{text}</option>)}</select></label>{"showCurrentChampion" in draft ? <Toggle label="显示当前英雄" description="只使用当前 payload 中的 champion_name。" checked={draft.showCurrentChampion} disabled={busy} onChange={(value) => updateDraft(kind, { showCurrentChampion: value })}/> : null}</section><section className="space-y-2 rounded-xl border border-cs2-border-subtle p-3"><b className="text-xs">可证明指标</b>{options.length ? options.map(([key, text, hint]) => <Toggle key={key} label={text} description={hint} checked={draft.display[key]} disabled={busy} onChange={(value) => updateDraft(kind, { display: { [key]: value } })}/>) : <p className="text-xs text-cs2-text-muted">组排关系由已有 `premade_group` 直接生成，不需要额外猜测指标。</p>}</section></div>
      <div className="rounded-xl border border-cs2-border-subtle p-3"><div className="flex items-center justify-between gap-2"><b className="text-xs">生成预览</b><span className="text-[10px] text-cs2-text-muted">最多发送前 10 行 · 当前 {lines.length} 行</span></div><textarea aria-label={`${label}生成预览`} readOnly value={lines.join("\n")} placeholder="读取玩家后点击“生成预览”；未选择指标时不会虚构数据。" rows={Math.min(10, Math.max(4, lines.length || 4))} className="mt-2 w-full resize-y rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 font-mono text-xs"/></div>
      <ShortcutGrid kind={kind} settings={settings} busy={busy} enabled={enabled} onSave={(target, value) => saveGeneratedShortcut(kind, target, value)}/>
    </div>;
  };

  return <section className="rounded-2xl border border-indigo-400/20 bg-cs2-bg-elevated p-4">
    <div className="flex flex-wrap items-center gap-3"><Keyboard className="h-4 w-4 text-indigo-300"/><div className="mr-auto"><h3 className="text-sm font-bold">游戏内预设</h3><p className="mt-1 text-xs text-cs2-text-muted">Rating、打野画像、组排关系和固定文字四个预设页。生成内容只使用当前已读取的 LCU payload；发送默认关闭，并受账号写入 gate 保护。</p></div><button role="switch" aria-label="启用游戏内预设发送" aria-checked={Boolean(settings?.in_game_send_enabled)} disabled={!settings?.toolkit_account_actions_enabled || busy} onClick={() => toggle(!settings?.in_game_send_enabled)} className={`relative h-6 w-11 rounded-full ${settings?.in_game_send_enabled ? "bg-indigo-500" : "bg-cs2-bg-input"} disabled:opacity-40`}><span className={`absolute left-1 top-1 h-4 w-4 rounded-full bg-white transition-transform ${settings?.in_game_send_enabled ? "translate-x-5" : ""}`}/></button></div>
    <div className="mt-3 flex flex-wrap items-center gap-2"><label className="text-xs text-cs2-text-muted">逐行间隔 <input aria-label="逐行发送间隔" type="number" min="100" max="5000" value={settings?.in_game_send_interval_ms || 250} disabled={!settings?.toolkit_account_actions_enabled || busy} onChange={(event) => onSettingsUpdate({ in_game_send_interval_ms: Number(event.target.value) })} className="ml-1 w-20 rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5"/> ms</label><button onClick={cancelSend} disabled={!enabled || busy} className="inline-flex items-center gap-1 rounded-lg border border-rose-400/25 bg-rose-400/10 px-3 py-1.5 text-xs text-rose-200 disabled:opacity-40"><X className="h-3.5 w-3.5"/>取消当前发送</button></div>
    <div role="tablist" aria-label="游戏内预设类型" className="mt-4 flex flex-wrap gap-1 border-b border-cs2-border-subtle">{[["rating", "Rating"], ["jungle", "打野画像"], ["premade", "组排关系"], ["fixed", "固定文字"]].map(([value, label]) => <button key={value} role="tab" aria-selected={tab === value} onClick={() => setTab(value)} className={`rounded-t-lg px-3 py-2 text-xs font-semibold ${tab === value ? "bg-indigo-400/15 text-indigo-200" : "text-cs2-text-muted hover:bg-white/5"}`}>{label}</button>)}</div>
    <div className="mt-4">{tab === "rating" ? renderGenerated("rating", "Rating", "对齐 LeagueAkari 的 Rating 指标：胜率、KDA、单杀、视野、团队输出/承伤/经济占比、补刀、参团、伤害经济效率、主力英雄和主位置。缺少完整团队或挑战字段时对应值显示为 —，不会补造。", RATING_OPTIONS) : null}{tab === "jungle" ? renderGenerated("jungle", "打野画像", "对齐 LeagueAkari 的六项打野预设：活动区域、首开营地、早期参与、团队首龙与小龙、团队史诗野怪以及历史主力打野英雄。缺失的时间线证据显示为 —，不会补造。", JUNGLE_OPTIONS) : null}{tab === "premade" ? renderGenerated("premade", "组排关系", "根据当前 payload 的 premade_group 生成组排关系；没有可证明的组排关系时保持空预览，不猜测队友关系。", []) : null}{tab === "fixed" ? <div className="space-y-3"><div className="flex flex-wrap items-center gap-2"><button disabled={!settings?.toolkit_account_actions_enabled || busy || items.length >= 100} onClick={add} className="inline-flex items-center gap-1 rounded-lg border border-indigo-400/25 bg-indigo-400/10 px-3 py-1.5 text-xs text-indigo-200 disabled:opacity-40"><Plus className="h-3.5 w-3.5"/>新增预设</button><button disabled={!settings?.toolkit_account_actions_enabled || busy} onClick={() => persist(items)} className="inline-flex items-center gap-1 rounded-lg border border-cs2-border px-3 py-1.5 text-xs disabled:opacity-40"><Save className="h-3.5 w-3.5"/>保存全部</button></div><div className="space-y-2">{items.map((item, index) => <article key={item.id} className="grid gap-2 rounded-xl border border-cs2-border-subtle p-3 lg:grid-cols-[180px_160px_1fr_auto]"><input aria-label={`预设标题 ${item.id}`} value={item.title} maxLength={64} onChange={(event) => patchItem(item.id, { title: event.target.value })} placeholder="预设标题" className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><input aria-label={`预设快捷键 ${item.id}`} value={item.shortcut || ""} maxLength={80} onChange={(event) => patchItem(item.id, { shortcut: event.target.value.trim() || null })} placeholder="可选，如 Ctrl+Alt+H" className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><textarea aria-label={`预设内容 ${item.id}`} value={item.content} maxLength={65536} rows={2} onChange={(event) => patchItem(item.id, { content: event.target.value })} placeholder="每行一条，最多发送前 10 行" className="resize-y rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><div className="flex items-center gap-1"><button aria-label={`上移 ${item.title}`} disabled={busy || index === 0} onClick={() => move(item.id, -1)} className="rounded-lg border border-cs2-border p-2 disabled:opacity-30"><ArrowUp className="h-3.5 w-3.5"/></button><button aria-label={`下移 ${item.title}`} disabled={busy || index === items.length - 1} onClick={() => move(item.id, 1)} className="rounded-lg border border-cs2-border p-2 disabled:opacity-30"><ArrowDown className="h-3.5 w-3.5"/></button><button aria-label={`发送 ${item.title}`} disabled={!enabled || busy || !item.content.trim()} onClick={() => sendFixed(item.id)} className="rounded-lg border border-emerald-400/25 bg-emerald-400/10 p-2 text-emerald-200 disabled:opacity-40"><Send className="h-3.5 w-3.5"/></button><button aria-label={`删除 ${item.title}`} disabled={busy} onClick={() => remove(item.id)} className="rounded-lg border border-rose-400/25 bg-rose-400/10 p-2 text-rose-200 disabled:opacity-40"><Trash2 className="h-3.5 w-3.5"/></button></div></article>)}{!items.length ? <div className="rounded-xl border border-dashed border-cs2-border p-6 text-center text-xs text-cs2-text-muted">尚未创建固定文字预设</div> : null}</div></div> : null}</div>
  </section>;
}
