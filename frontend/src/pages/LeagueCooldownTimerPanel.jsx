import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ChevronRight, Pin, PinOff, Timer, X } from "lucide-react";
import { fetchLeagueCooldownTimerState, fetchLeagueLabStatus, saveLeagueLabSettings, sendLeagueCooldownTimerText } from "../api/leagueLabApi";
import { getLeagueChampionIconUrl, getLeagueSummonerSpellIconUrl } from "../api/api";
import {
  adjustLeagueSpellTimer,
  buildLeagueTimerChatText,
  createLeagueSpellTimer,
  formatLeagueSpellTimer,
} from "../utils/leagueCooldownTimer";

function SpellButton({ player, spell, timer, timerType, abilityHaste, reverseAdjustment, now, onChange, onSend }) {
  const lastRightClick = useRef(0);
  const toggle = () => {
    if (timer) return onChange(null);
    onChange(createLeagueSpellTimer({ timerType, cooldownSeconds: spell?.cooldown, abilityHaste }));
  };
  const wheel = (event) => {
    event.preventDefault();
    if (!timer) return;
    onChange(adjustLeagueSpellTimer(timer, event.deltaY, reverseAdjustment));
  };
  const rightClick = (event) => {
    event.preventDefault();
    const at = Date.now();
    if (at - lastRightClick.current < 360) onSend();
    lastRightClick.current = at;
  };
  return <button onClick={toggle} onWheel={wheel} onContextMenu={rightClick} title={`${spell?.name || "自定义计时"}：单击开始/清除，滚轮校时，双击右键发送`} className="relative h-9 w-9 overflow-hidden rounded border border-white/20 bg-black/40 active:scale-110">
    {spell?.id ? <img src={getLeagueSummonerSpellIconUrl(spell.id)} alt={spell.name || ""} className="h-full w-full object-cover"/> : <Timer className="m-2 h-5 w-5 text-zinc-300"/>}
    {timer&&<span className={`absolute inset-0 grid place-items-center bg-black/65 text-[11px] font-black ${formatLeagueSpellTimer(timer,now)==="OK"?"text-fuchsia-200":"text-white"}`}>{formatLeagueSpellTimer(timer,now)}</span>}
  </button>;
}

export default function LeagueCooldownTimerPanel() {
  const [state,setState]=useState(null),[timers,setTimers]=useState({}),[now,setNow]=useState(Date.now()),[error,setError]=useState(""),[settings,setSettings]=useState(null),[pinned,setPinned]=useState(true);
  const load=useCallback(async()=>{try{setState(await fetchLeagueCooldownTimerState());setError("");}catch(e){setError(e?.response?.data?.detail||"等待游戏数据");}},[]);
  useEffect(()=>{load();const timer=setInterval(load,1500);return()=>clearInterval(timer);},[load]);
  useEffect(()=>{const timer=setInterval(()=>setNow(Date.now()),100);return()=>clearInterval(timer);},[]);
  useEffect(()=>setTimers({}),[state?.timer_type,state?.game_mode]);
  useEffect(()=>{let disposed=false;fetchLeagueLabStatus().then((status)=>{if(disposed)return;const next=status?.settings?.cooldown_pinned!==false;setSettings(status?.settings||null);setPinned(next);invoke("set_league_window_pinned",{kind:"cooldown",pinned:next}).catch(()=>{});}).catch(()=>{});return()=>{disposed=true;};},[]);
  const setWindowPinned=async()=>{if(!settings)return;const next=!pinned;try{await invoke("set_league_window_pinned",{kind:"cooldown",pinned:next});const result=await saveLeagueLabSettings({...settings,cooldown_pinned:next});if(result?.settings)setSettings(result.settings);setPinned(next);}catch{setError("窗口置顶设置失败");}};
  const rows=state?.players?.length?state.players:Array.from({length:5},(_,index)=>({puuid:`placeholder-${index}`,champion_id:0,spell1_id:0,spell2_id:0}));
  const send=async(player,spell,timer)=>{
    if(!timer||state?.game_time==null||!player?.champion_id||!spell?.id)return;
    const text=buildLeagueTimerChatText({playerName:player.champion_name,spellName:spell.name,timer,gameTimeSeconds:state.game_time});
    if(!text)return;
    try{await sendLeagueCooldownTimerText(text);setError("");}catch(e){setError(e?.response?.data?.detail||"发送失败");}
  };
  return <div className="min-h-screen w-fit min-w-[112px] select-none overflow-hidden rounded-lg bg-[#151518]/90 text-white shadow-2xl">
    <header data-tauri-drag-region className="flex h-7 items-center justify-between border-b border-white/10 px-2 text-[10px] text-zinc-400"><span data-tauri-drag-region>敌方技能</span><span className="flex items-center gap-1"><button type="button" aria-label={pinned ? "取消置顶" : "窗口置顶"} onClick={setWindowPinned} className={`rounded p-1 hover:bg-white/10 ${pinned ? "text-emerald-300" : "text-zinc-500"}`}>{pinned ? <Pin className="h-3 w-3"/> : <PinOff className="h-3 w-3"/>}</button><button type="button" aria-label="关闭技能计时器" onClick={()=>getCurrentWindow().close()} className="rounded p-1 hover:bg-white/10"><X className="h-3 w-3"/></button></span></header>
    <div className="space-y-1 p-2">{rows.slice(0,5).map((player,index)=>{const spell1=state?.spells?.[player.spell1_id],spell2=state?.spells?.[player.spell2_id];return <div key={player.puuid||index} className="flex items-center gap-1">
      {player.champion_id?<img src={getLeagueChampionIconUrl(player.champion_id)} alt={player.champion_name||""} title={player.champion_name||""} className="h-9 w-9 rounded border-2 border-fuchsia-300/45 object-cover"/>:<div className="grid h-9 w-9 place-items-center rounded border border-cyan-300/25"><Timer className="h-5 w-5 text-cyan-300/40"/></div>}
      <ChevronRight className="h-3 w-3 text-white/35"/>
      <SpellButton player={player} spell={spell1} timer={timers[`${player.puuid}:1`]} timerType={state?.timer_type||"countdown"} abilityHaste={state?.ability_haste} reverseAdjustment={state?.reverse_adjustment} now={now} onChange={(value)=>setTimers((current)=>({...current,[`${player.puuid}:1`]:value}))} onSend={()=>send(player,spell1,timers[`${player.puuid}:1`])}/>
      <SpellButton player={player} spell={spell2} timer={timers[`${player.puuid}:2`]} timerType={state?.timer_type||"countdown"} abilityHaste={state?.ability_haste} reverseAdjustment={state?.reverse_adjustment} now={now} onChange={(value)=>setTimers((current)=>({...current,[`${player.puuid}:2`]:value}))} onSend={()=>send(player,spell2,timers[`${player.puuid}:2`])}/>
    </div>})}</div>
    {error&&<div className="max-w-[160px] px-2 pb-2 text-[9px] leading-3 text-amber-200">{error}</div>}
  </div>;
}
