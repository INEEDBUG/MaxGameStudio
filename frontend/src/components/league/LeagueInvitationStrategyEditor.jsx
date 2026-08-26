import { useMemo, useState } from "react";
import { Plus, X } from "lucide-react";

const QUEUE_TYPES = [
  ["<DEFAULT>", "默认策略"],
  ["RANKED_SOLO_5x5", "单双排"],
  ["RANKED_FLEX_SR", "灵活排位"],
  ["NORMAL", "匹配模式"],
  ["ARAM_UNRANKED_5x5", "极地大乱斗"],
  ["KIWI", "特殊大乱斗"],
  ["CHERRY", "斗魂竞技场"],
  ["URF", "无限火力"],
  ["NORMAL_TFT", "云顶之弈匹配"],
  ["RANKED_TFT", "云顶之弈排位"],
  ["RANKED_TFT_TURBO", "云顶之弈狂暴"],
  ["RANKED_TFT_DOUBLE_UP", "云顶之弈双人作战"],
];

export default function LeagueInvitationStrategyEditor({ strategies = {}, fallback = "ignore", onChange }) {
  const normalized={"<DEFAULT>":strategies["<DEFAULT>"]||fallback,...strategies};
  const [candidate,setCandidate]=useState("");
  const labels=useMemo(()=>new Map(QUEUE_TYPES),[]);
  const available=QUEUE_TYPES.filter(([key])=>!(key in normalized));
  const setStrategy=(key,value)=>onChange({...normalized,[key]:value});
  const remove=(key)=>{const next={...normalized};delete next[key];onChange(next);};
  const add=()=>{if(!candidate)return;onChange({...normalized,[candidate]:"ignore"});setCandidate("");};
  return <div className="space-y-2 p-4"><div className="text-xs font-semibold text-cs2-text-secondary">按对局类型设置邀请策略</div>{Object.entries(normalized).map(([key,value])=><div key={key} className="flex flex-wrap items-center gap-3 rounded-xl border border-cs2-border-subtle px-3 py-2"><span className="min-w-[150px] flex-1 text-sm font-semibold">{labels.get(key)||key}</span><div className="flex gap-1">{[["accept","接受"],["decline","拒绝"],["ignore","不处理"]].map(([id,label])=><button key={id} onClick={()=>setStrategy(key,id)} className={`rounded-lg px-3 py-1.5 text-xs ${value===id?"bg-emerald-400/15 font-semibold text-emerald-300":"text-cs2-text-muted hover:bg-white/5"}`}>{label}</button>)}</div>{key!=="<DEFAULT>"&&<button onClick={()=>remove(key)} aria-label="移除类型" className="rounded-lg p-1.5 text-red-300 hover:bg-red-400/10"><X className="h-4 w-4"/></button>}</div>)}{available.length>0&&<div className="flex gap-2 pt-1"><select value={candidate} onChange={(event)=>setCandidate(event.target.value)} className="min-w-0 flex-1 rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-sm"><option value="">添加对局类型…</option>{available.map(([key,label])=><option key={key} value={key}>{label}</option>)}</select><button onClick={add} disabled={!candidate} className="inline-flex items-center gap-1 rounded-lg border border-emerald-400/30 px-3 py-2 text-xs font-semibold text-emerald-300 disabled:opacity-40"><Plus className="h-4 w-4"/>添加</button></div>}</div>;
}
