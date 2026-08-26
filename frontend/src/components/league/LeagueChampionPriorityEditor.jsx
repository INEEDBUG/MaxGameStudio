import { useEffect, useMemo, useState } from "react";
import { ArrowDown, ArrowUp, Search, X } from "lucide-react";
import { fetchLeagueChampions } from "../../api/leagueLabApi";
import { getLeagueChampionIconUrl } from "../../api/api";

const MODES = [["ranked","排位模式"],["normal","普通 / 快速 / 赛事"],["aram","大乱斗类"],["cherry","斗魂竞技场"],["urf","无限火力"],["oneforall","克隆模式"],["ultbook","终极魔典"],["bot","人机 / 特殊 PvE"],["custom","自定义对局"],["default","其他模式"]];
const POSITIONS = [["default","通用"],["top","上路"],["jungle","打野"],["middle","中路"],["bottom","下路"],["utility","辅助"]];
const ROLE_FILTERS = [["all","全部定位"],["fighter","战士"],["tank","坦克"],["mage","法师"],["assassin","刺客"],["marksman","射手"],["support","辅助"]];
const AUTO_SELECT_MOVE_LABELS = {
  "pick-intent": "预选英雄",
  "show-pick": "亮出英雄",
  "complete-pick": "锁定英雄",
  "show-ban": "亮出禁用",
  "complete-ban": "锁定禁用",
  vote: "投票",
  "show-subset-pick": "亮出子集英雄",
  "complete-subset-pick": "锁定子集英雄",
  "subset-bench-swap": "子集备战席换位",
  "bench-swap": "备战席换位",
};
const AUTO_SELECT_PLAN_LABELS = {
  delayed_pick: "选择计划",
  delayed_ban: "禁用计划",
  delayed_bench_swap: "备战席计划",
  delayed_trade: "换英雄计划",
};
const emptyPool = () => ({default:[],top:[],jungle:[],middle:[],bottom:[],utility:[]});
// LeagueAkari's empty mode profile is deliberately inert: no delay, no
// intent/bench/trade side effects until the user explicitly enables them.
const emptyProfile = () => ({pick:{enabled:false,champions:emptyPool(),delay_seconds:0,ignore_intent:false,strategy:"show-and-lock-in",show_intent:false,bench_select_first_available_champion:false,bench_swap_accumulated_delay_seconds:2.9,bench_handle_trade_enabled:false},ban:{enabled:false,champions:emptyPool(),delay_seconds:0,strategy:"show-and-lock-in"}});

function Toggle({checked,onChange}) { return <button type="button" role="switch" aria-checked={checked} onClick={()=>onChange(!checked)} className={`relative h-6 w-11 shrink-0 rounded-full ${checked?"bg-emerald-500":"bg-cs2-bg-input"}`}><span className={`absolute left-1 top-1 h-4 w-4 rounded-full bg-white transition-transform ${checked?"translate-x-5":""}`}/></button>; }

function RuntimeAutoSelectPreview({status}) {
  const autoSelect = status?.auto_select;
  if (status?.phase !== "ChampSelect" || !autoSelect || typeof autoSelect !== "object") return null;
  const move = autoSelect.move;
  const plans = Object.entries(AUTO_SELECT_PLAN_LABELS)
    .map(([key, label]) => ({ key, label, task: autoSelect[key] }))
    .filter(({ task }) => task && typeof task === "object");
  const config = autoSelect.config || {};
  const enabled = autoSelect.enabled === true;
  const actionability = autoSelect.actionability || {};
  const actionabilityKey = move === "pick-intent" ? "intent" : move === "vote" ? "vote" : move === "bench-swap" || move === "subset-bench-swap" ? "bench_swap" : move?.includes("ban") ? (move.startsWith("show") ? "show" : "complete") : move?.includes("pick") ? (move.startsWith("show") ? "show" : "complete") : null;
  const actionabilityState = actionabilityKey && typeof actionability[actionabilityKey] === "boolean" ? actionability[actionabilityKey] : null;
  const expectedKey = move === "show-ban" || move === "complete-ban" ? "expected_bans" : move === "bench-swap" || move === "subset-bench-swap" ? "expected_swaps" : "expected_picks";
  const expected = Array.isArray(autoSelect[expectedKey]) ? autoSelect[expectedKey].slice(0, 8) : [];
  return <section data-testid="main-auto-select-runtime" className="mb-4 rounded-2xl border border-cyan-400/20 bg-cyan-500/[.045] p-4 lg:col-span-2" aria-live="polite">
    <div className="flex flex-wrap items-center justify-between gap-2"><div><div className="text-xs font-bold uppercase tracking-[.16em] text-cyan-300">客户端自动选禁状态</div><div className="mt-1 text-sm font-semibold text-cs2-text-primary">{AUTO_SELECT_MOVE_LABELS[move] || (move ? `未知动作 · ${move}` : "等待客户端动作")}</div></div><span className={`rounded-full border border-white/10 px-2 py-1 text-[10px] font-semibold ${enabled && !autoSelect.temporarily_disabled ? "text-emerald-200" : "text-amber-200"}`}>{autoSelect.temporarily_disabled ? "临时暂停" : enabled ? "自动选禁已启用" : "仅展示状态"}</span></div>
    <div className="mt-3 flex flex-wrap gap-1">{Object.entries(AUTO_SELECT_MOVE_LABELS).map(([id, label]) => <span key={id} className={`rounded-full border px-2 py-1 text-[10px] ${id === move ? "border-cyan-300/50 bg-cyan-300/10 text-cyan-100" : "border-white/10 text-cs2-text-muted"}`}>{label}</span>)}</div>
    <div className="mt-3 grid gap-2 text-[11px] text-cs2-text-muted sm:grid-cols-2"><div>模式：<b className="text-cs2-text-secondary">{autoSelect.active_group_id || "未知"}</b> · 分路：<b className="text-cs2-text-secondary">{autoSelect.assigned_position || "未知"}</b></div><div>动作通道：<b className={actionabilityState === true ? "text-emerald-200" : actionabilityState === false ? "text-amber-200" : "text-cs2-text-secondary"}>{actionabilityState === true ? "可执行" : actionabilityState === false ? "当前不可执行" : "仅展示 / 等待客户端状态"}</b></div></div>
    {expected.length > 0 && <div className="mt-3 rounded-xl border border-white/10 bg-black/10 p-3"><div className="mb-2 text-[10px] font-semibold text-cs2-text-secondary">当前候选状态</div><div className="flex flex-wrap gap-1">{expected.map((row) => <span key={String(row.id)} className="rounded-full border border-white/10 px-2 py-1 text-[10px] text-cs2-text-muted">#{row.id} · {row.status || "未知"}</span>)}</div></div>}
    {plans.length > 0 && <div className="mt-3 grid gap-2 sm:grid-cols-2">{plans.map(({key,label,task}) => <div key={key} className="rounded-xl border border-white/10 bg-black/10 p-3 text-[10px]"><div className="flex items-center justify-between gap-2"><span className="font-semibold text-cs2-text-secondary">{label}</span><span className="tabular-nums text-cyan-100">{task.remaining_seconds != null ? `${Number(task.remaining_seconds).toFixed(1)} 秒` : "等待客户端倒计时"}</span></div><div className="mt-1 text-cs2-text-muted">{task.move || task.operation || task.action || "等待执行"}{task.completed === true ? " · 已完成动作" : ""}</div></div>)}</div>}
    {!plans.length && <div className="mt-3 text-[10px] text-cs2-text-muted">当前没有客户端返回的延迟计划，不显示伪造倒计时。</div>}
    <div className="mt-3 text-[10px] text-cs2-text-muted">策略：选择 {config.pick_strategy || "show-and-lock-in"} · 禁用 {config.ban_strategy || "show-and-lock-in"} · 预选 {config.show_intent ? "开启" : "关闭"}</div>
  </section>;
}

export default function LeagueChampionPriorityEditor({settings,onUpdate,status}) {
  const [mode,setMode]=useState("aram"),[position,setPosition]=useState("default"),[kind,setKind]=useState("pick"),[picker,setPicker]=useState(false),[search,setSearch]=useState(""),[roleFilter,setRoleFilter]=useState("all"),[champions,setChampions]=useState([]);
  useEffect(()=>{fetchLeagueChampions().then((data)=>setChampions(data.champions||[])).catch(()=>setChampions([]));},[]);
  const profiles=settings.auto_select_profiles||{};
  const profile=profiles[mode]||emptyProfile();
  const config=profile[kind];
  const selected=config.champions?.[position]||[];
  const selectableChampions=useMemo(()=>mode==="cherry"&&kind==="pick"?[{id:-3,name:"勇敢举动",alias:"bravery",special:true},...champions]:champions,[champions,mode,kind]);
  const byId=useMemo(()=>new Map(selectableChampions.map((item)=>[item.id,item])),[selectableChampions]);
  const filtered=useMemo(()=>{const q=search.trim().toLowerCase();return selectableChampions.filter((item)=>(roleFilter==="all"||item.special||(item.roles||[]).includes(roleFilter))&&(!q||item.name.toLowerCase().includes(q)||item.alias.toLowerCase().includes(q)||String(item.id).includes(q)));},[selectableChampions,search,roleFilter]);
  const patchConfig=(patch)=>onUpdate({auto_select_profiles:{...profiles,[mode]:{...profile,[kind]:{...config,...patch}}}});
  const setPool=(ids)=>patchConfig({champions:{...emptyPool(),...(config.champions||{}),[position]:ids}});
  const move=(index,delta)=>{const next=[...selected],target=index+delta;if(target<0||target>=next.length)return;[next[index],next[target]]=[next[target],next[index]];setPool(next);};
  return <div className="grid gap-4 lg:grid-cols-[220px_1fr]">
    <RuntimeAutoSelectPreview status={status}/>
    <aside className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-3"><div className="mb-2 text-xs font-bold text-cs2-text-muted">生效模式</div>{MODES.map(([id,label])=><button key={id} onClick={()=>setMode(id)} className={`mb-1 w-full rounded-lg px-3 py-2 text-left text-sm ${mode===id?"bg-emerald-400/15 font-semibold text-emerald-300":"text-cs2-text-secondary hover:bg-white/5"}`}>{label}</button>)}</aside>
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
      <div className="mb-4 flex gap-2">{[["pick","英雄选择"],["ban","英雄禁用"]].map(([id,label])=><button key={id} onClick={()=>setKind(id)} className={`rounded-lg px-4 py-2 text-xs font-semibold ${kind===id?"bg-emerald-500/15 text-emerald-300":"text-cs2-text-muted"}`}>{label}</button>)}</div>
      <div className="mb-4 flex items-center justify-between border-b border-cs2-border-subtle pb-4"><div><div className="font-semibold">启用此模式的自动{kind==="pick"?"选择":"禁用"}</div><div className="mt-1 text-xs text-cs2-text-muted">按列表顺序选择第一个当前可用英雄。</div></div><Toggle checked={config.enabled} onChange={(enabled)=>patchConfig({enabled})}/></div>
      <div className="mb-3 flex flex-wrap gap-2">{POSITIONS.map(([id,label])=><button key={id} onClick={()=>setPosition(id)} className={`rounded-lg border px-3 py-1.5 text-xs ${position===id?"border-emerald-400/40 bg-emerald-400/10 text-emerald-300":"border-cs2-border text-cs2-text-muted"}`}>{label}</button>)}</div>
      <button onClick={()=>setPicker(true)} className="mb-3 rounded-lg border border-emerald-400/30 bg-emerald-400/10 px-3 py-2 text-xs font-semibold text-emerald-300">选择并排序英雄（{selected.length}）</button>
      <div className="space-y-1">{selected.map((id,index)=>{const champion=byId.get(id);return <div key={`${id}-${index}`} className="flex items-center gap-3 rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2">{champion?.special?<span className="grid h-8 w-8 place-items-center rounded-md bg-amber-400/10 text-xs font-bold text-amber-300">勇</span>:<img src={getLeagueChampionIconUrl(id)} alt="" className="h-8 w-8 rounded-md bg-white/5 object-cover"/>}<span className="min-w-0 flex-1 truncate text-sm">{champion?.name||`英雄 ${id}`}<small className="ml-2 text-[10px] text-cs2-text-muted">{(champion?.roles||[]).join(" / ")}</small></span><button onClick={()=>move(index,-1)} aria-label="上移"><ArrowUp className="h-4 w-4"/></button><button onClick={()=>move(index,1)} aria-label="下移"><ArrowDown className="h-4 w-4"/></button><button onClick={()=>setPool(selected.filter((_,i)=>i!==index))} aria-label="移除"><X className="h-4 w-4 text-red-300"/></button></div>})}</div>
      <div className="mt-4 grid gap-3 md:grid-cols-2"><label className="text-xs text-cs2-text-muted">执行延迟（秒）<input type="number" min="0" step="0.5" value={config.delay_seconds} onChange={(e)=>patchConfig({delay_seconds:Number(e.target.value)})} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-sm"/></label><label className="text-xs text-cs2-text-muted">锁定策略<select value={config.strategy} onChange={(e)=>patchConfig({strategy:e.target.value})} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-sm"><option value="just-show">仅亮出</option><option value="show-and-lock-in">亮出后延迟锁定</option><option value="lock-in-immediately">延迟后直接锁定</option></select></label></div>
      {kind==="pick"&&<div className="mt-4 grid gap-2 text-sm">{[["show_intent","提前预选"],["ignore_intent","忽略队友预选冲突"],["bench_select_first_available_champion","优先选择备战席"],["bench_handle_trade_enabled","自动处理英雄交换请求"]].map(([key,label])=><div key={key} className="flex items-center justify-between rounded-lg border border-cs2-border px-3 py-2"><span>{label}</span><Toggle checked={Boolean(config[key])} onChange={(value)=>patchConfig({[key]:value})}/></div>)}<label className="text-xs text-cs2-text-muted">备战席最短累积等待（秒）<input type="number" min="0" step="0.1" value={config.bench_swap_accumulated_delay_seconds} onChange={(e)=>patchConfig({bench_swap_accumulated_delay_seconds:Number(e.target.value)})} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-sm"/></label></div>}
    </section>
    {picker&&<div className="fixed inset-0 z-[100] grid place-items-center bg-black/65 p-6" onMouseDown={()=>setPicker(false)}><div className="grid max-h-[78vh] w-full max-w-3xl grid-cols-2 overflow-hidden rounded-2xl border border-cs2-border bg-cs2-bg-page shadow-2xl" onMouseDown={(e)=>e.stopPropagation()}><div className="border-r border-cs2-border p-4"><div className="mb-2 flex items-center gap-2 rounded-lg border border-cs2-border bg-cs2-bg-input px-3"><Search className="h-4 w-4 text-cs2-text-muted"/><input autoFocus value={search} onChange={(e)=>setSearch(e.target.value)} placeholder="搜索英雄" className="w-full bg-transparent py-2.5 text-sm outline-none"/></div><select aria-label="英雄定位筛选" value={roleFilter} onChange={(e)=>setRoleFilter(e.target.value)} className="mb-3 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs">{ROLE_FILTERS.map(([id,label])=><option key={id} value={id}>{label}</option>)}</select><div className="max-h-[55vh] overflow-y-auto">{filtered.map((champion)=><button key={champion.id} onClick={()=>!selected.includes(champion.id)&&setPool([...selected,champion.id])} className={`flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-sm ${selected.includes(champion.id)?"text-emerald-300":"hover:bg-white/5"}`}>{champion.special?<span className="grid h-8 w-8 place-items-center rounded-md bg-amber-400/10 text-xs">勇</span>:<img src={getLeagueChampionIconUrl(champion.id)} alt="" className="h-8 w-8 rounded-md bg-white/5 object-cover"/>}<span className="min-w-0 flex-1"><b>{champion.name}</b><small className="ml-2 text-[10px] text-cs2-text-muted">{(champion.roles||[]).join(" / ")}</small></span></button>)}</div></div><div className="p-4"><div className="mb-3 flex items-center justify-between"><div className="font-semibold">已选 {selected.length} 项</div><button onClick={()=>setPicker(false)}><X className="h-5 w-5"/></button></div><div className="max-h-[60vh] space-y-1 overflow-y-auto">{selected.map((id,index)=><div key={`${id}-${index}`} className="flex items-center gap-2 rounded-lg bg-white/5 px-3 py-2 text-sm"><span className="flex-1">{byId.get(id)?.name||id}</span><button onClick={()=>move(index,-1)}><ArrowUp className="h-4 w-4"/></button><button onClick={()=>move(index,1)}><ArrowDown className="h-4 w-4"/></button><button onClick={()=>setPool(selected.filter((_,i)=>i!==index))}><X className="h-4 w-4"/></button></div>)}</div></div></div></div>}
  </div>;
}
