import { useEffect, useRef, useState } from "react";
import { AlertTriangle, AppWindow, Boxes, Gift, Maximize2, MessageSquare, PlayCircle, Power, RefreshCw, ScrollText, Settings2, Trophy, Users } from "lucide-react";
import { fetchLeagueClientWindow, fetchLeagueGameSettingsFile, fetchLeagueLabStatus, fetchLeagueOngoingGame, fetchLeagueToolkitOverview, resizeLeagueClientWindow, runLeagueLabAction, sendLeagueInGameLines, terminateLeagueGameClient, updateLeagueChatPresence, updateLeagueGameSettingsFile, updateLeagueRankedStatus } from "../../api/leagueLabApi";
import { buildLeagueFormPreset, buildLeagueJunglePreset, buildLeaguePremadePreset } from "../../utils/leagueChatPresets";
import LeagueAccountTools from "./LeagueAccountTools";
import LeagueAdvancedToolkit from "./LeagueAdvancedToolkit";
import LeagueInGamePresetTools from "./LeagueInGamePresetTools";
import LeagueAuxShortcutSettings from "./LeagueAuxShortcutSettings";

const cards = [
  ["missions", "任务", ScrollText],
  ["unclaimed_rewards", "待处理奖励", Gift],
  ["loot", "战利品种类", Boxes],
  ["friends", "好友", Users],
];
const rankedQueues = [["RANKED_SOLO_5x5","单双排位"],["RANKED_FLEX_SR","灵活排位"],["RANKED_TFT","云顶之弈"]];
const rankedTiers = ["IRON","BRONZE","SILVER","GOLD","PLATINUM","EMERALD","DIAMOND","MASTER","GRANDMASTER","CHALLENGER"];
const TOOLKIT_TABS = [
  ["client", "客户端", AppWindow],
  ["in-game-send", "游戏内发送", MessageSquare],
  ["in-process", "进行中", PlayCircle],
  ["lobby", "房间", Users],
  ["misc", "其他", Settings2],
  ["claim-tools", "领取奖励", Gift],
  ["friend-tools", "好友", Users],
];

function initialToolkitTab() {
  try {
    const value = new URLSearchParams(window.location.search).get("toolkit");
    return TOOLKIT_TABS.some(([id]) => id === value) ? value : "client";
  } catch {
    return "client";
  }
}

function InProcessTools({ status, enabled, busy, onBusyChange, onError, onRefresh }) {
  const phase = status?.phase || "Unknown";
  const run = async (action) => {
    onBusyChange(true);
    try { await runLeagueLabAction(action); await onRefresh(); }
    catch (error) { onError(error?.response?.data?.detail || error?.message || "操作失败"); }
    finally { onBusyChange(false); }
  };
  const endgame = ["WaitingForStats", "PreEndOfGame", "EndOfGame"].includes(phase);
  return <section data-testid="league-toolkit-in-process" className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
    <h3 className="text-sm font-bold">进行中的游戏流程</h3>
    <p className="mt-1 text-xs text-cs2-text-muted">当前阶段：{phase}。与 LeagueAkari 的 InProcess 工具一致，仅在已证明的客户端阶段显示可用操作。</p>
    <div className="mt-4 grid gap-2 sm:grid-cols-2">
      <button type="button" disabled={!enabled || busy || !endgame} onClick={() => run("play-again")} title={!enabled ? "请先启用账号写入保护" : !endgame ? "仅结算阶段可用" : ""} className="rounded-xl border border-emerald-400/25 bg-emerald-400/10 px-3 py-2 text-left text-xs font-semibold text-emerald-200 disabled:opacity-40">返回房间<span className="mt-1 block text-[10px] font-normal text-cs2-text-muted">结算完成后再次进入房间</span></button>
      <button type="button" disabled={!enabled || busy || phase !== "Lobby"} onClick={() => { if (window.confirm("确认离开当前房间？")) run("leave-lobby"); }} title={!enabled ? "请先启用账号写入保护" : phase !== "Lobby" ? "仅房间阶段可用" : ""} className="rounded-xl border border-rose-400/25 bg-rose-400/10 px-3 py-2 text-left text-xs font-semibold text-rose-200 disabled:opacity-40">离开房间<span className="mt-1 block text-[10px] font-normal text-cs2-text-muted">删除当前 LCU 房间</span></button>
    </div>
  </section>;
}

export default function LeagueToolkitOverview({ onError, settings, onSettingsUpdate, onDryRunGame, onOpenPlayer, streamerMode = Boolean(settings?.streamer_mode_enabled), useAliases = Boolean(settings?.streamer_mode_use_aliases) }) {
  const [data, setData] = useState(null);
  const [clientStatus, setClientStatus] = useState(null);
  const [busy, setBusy] = useState(false);
  const [availability,setAvailability]=useState("chat");
  const [statusMessage,setStatusMessage]=useState("");
  const [rankedStatus,setRankedStatus]=useState({queue:"RANKED_SOLO_5x5",tier:"CHALLENGER",division:"I"});
  const [preset,setPreset]=useState(()=>localStorage.getItem("league-fixed-chat-preset")||"");
  const [presetTarget,setPresetTarget]=useState("all");
  const [settingsFile,setSettingsFile]=useState(null);
  const [clientWindow,setClientWindow]=useState(null);
  const [toolkitTab, setToolkitTab] = useState(initialToolkitTab);
  const [windowSize,setWindowSize]=useState({baseWidth:1280,baseHeight:720});
  const loadRequest = useRef(0);
  const disposed = useRef(false);
  const load = async () => { const request=++loadRequest.current;setBusy(true); try { const [next,status]=await Promise.all([fetchLeagueToolkitOverview(),fetchLeagueLabStatus()]);if(disposed.current||request!==loadRequest.current)return;setData(next);setClientStatus(status);setAvailability(next?.chat_presence?.availability||"chat");setStatusMessage(next?.chat_presence?.statusMessage||"");setRankedStatus(status?.settings?.ranked_status||{queue:"RANKED_SOLO_5x5",tier:"CHALLENGER",division:"I"});try{const file=await fetchLeagueGameSettingsFile();if(!disposed.current&&request===loadRequest.current)setSettingsFile(file);}catch{if(!disposed.current&&request===loadRequest.current)setSettingsFile(null);}try{const nextWindow=await fetchLeagueClientWindow();if(!disposed.current&&request===loadRequest.current)setClientWindow(nextWindow);}catch{if(!disposed.current&&request===loadRequest.current)setClientWindow(null);} } catch (error) { if(!disposed.current&&request===loadRequest.current)onError(error?.response?.data?.detail || "工具箱读取失败"); } finally { if(!disposed.current&&request===loadRequest.current)setBusy(false); } };
  const applyPresence=async()=>{setBusy(true);try{const next=await updateLeagueChatPresence({availability,status_message:statusMessage});setData((current)=>({...current,chat_presence:next.chat_presence}));}catch(error){onError(error?.response?.data?.detail||"聊天状态应用失败");}finally{setBusy(false);}};
  const applyRanked=async()=>{setBusy(true);try{await updateLeagueRankedStatus(rankedStatus);}catch(error){onError(error?.response?.data?.detail||"排位展示应用失败");}finally{setBusy(false);}};
  const sendPreset=async()=>{const lines=preset.split(/\r?\n/).map((line)=>line.trim()).filter(Boolean).slice(0,10);if(window.prompt("该操作会向当前房间、英雄选择或前台游戏聊天发送这些内容。\n请输入“我确认发送”继续：")!=="我确认发送")return;localStorage.setItem("league-fixed-chat-preset",preset);setBusy(true);try{await sendLeagueInGameLines(lines,"我确认发送");}catch(error){onError(error?.response?.data?.detail||"预设消息发送失败");}finally{setBusy(false);}};
  const terminateGame=async()=>{const confirmation=window.prompt("只会结束当前前台的 League of Legends.exe，未保存的本局状态可能丢失。\n请输入“我确认结束游戏”继续：");if(confirmation!=="我确认结束游戏")return;setBusy(true);try{await terminateLeagueGameClient(confirmation);}catch(error){onError(error?.response?.data?.detail||"结束游戏进程失败");}finally{setBusy(false);}};
  const toggleSettingsFile=async()=>{if(!settings?.toolkit_account_actions_enabled){onError("请先在工具箱中启用账号写入保护");return;}const mode=settingsFile?.mode==="readonly"?"writable":"readonly";setBusy(true);try{setSettingsFile(await updateLeagueGameSettingsFile(mode));}catch(error){onError(error?.response?.data?.detail||"修改游戏设置文件属性失败");}finally{setBusy(false);}};
  const resizeClient=async()=>{if(!window.confirm(`将 League 客户端内容区调整为 ${windowSize.baseWidth} × ${windowSize.baseHeight} 的缩放尺寸并居中，确定继续吗？`))return;setBusy(true);try{setClientWindow(await resizeLeagueClientWindow(Number(windowSize.baseWidth),Number(windowSize.baseHeight)));}catch(error){onError(error?.response?.data?.detail||"调整 League 客户端窗口失败");}finally{setBusy(false);}};
  const generatePreset=async(type)=>{setBusy(true);try{const [game,status]=await Promise.all([fetchLeagueOngoingGame(),fetchLeagueLabStatus()]);const players=game.players||[];const own=players.find((player)=>player.puuid&&player.puuid===status?.current_summoner?.puuid);const selected=presetTarget==="all"||!own?players:players.filter((player)=>presetTarget==="friendly"?String(player.team)===String(own.team):String(player.team)!==String(own.team));const lines=type==="premade"?buildLeaguePremadePreset(selected):type==="jungle"?buildLeagueJunglePreset(selected):buildLeagueFormPreset(selected);if(!lines.length)throw new Error(type==="premade"?"当前没有可显示的组排关系":type==="jungle"?"当前没有可用的打野画像":"当前没有实时对局数据");setPreset(lines.slice(0,10).join("\n"));}catch(error){onError(error?.response?.data?.detail||error?.message||"生成消息草稿失败");}finally{setBusy(false);}};
  const toggleAccountActions=async()=>{const next=!settings?.toolkit_account_actions_enabled;if(!next&&!window.confirm("关闭后，工具箱将立即回到只读模式，所有账号写入按钮都会被禁用。确定关闭吗？"))return;if(next&&!window.confirm("启用后，工具箱将允许已确认的账号写入操作；敏感操作仍需二次输入确认短语，且不会随机执行。确定启用吗？"))return;setBusy(true);try{await onSettingsUpdate({toolkit_account_actions_enabled:next});await load();}finally{setBusy(false);}};
  useEffect(() => { disposed.current=false;void load();return()=>{disposed.current=true;loadRequest.current+=1;}; }, []);
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    params.set("toolkit", toolkitTab);
    window.history.replaceState(window.history.state, "", `${window.location.pathname}?${params.toString()}${window.location.hash}`);
  }, [toolkitTab]);
  useEffect(() => {
    const handlePopState = () => {
      try {
        const value = new URLSearchParams(window.location.search).get("toolkit");
        if (TOOLKIT_TABS.some(([id]) => id === value)) setToolkitTab(value);
      } catch { /* ignore malformed browser history entries */ }
    };
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);
  return <div className={`space-y-4 toolkit-overview-${toolkitTab}`}>
    <style>{`\n      .toolkit-overview-in-game-send > :not(:nth-child(2)):not(:nth-child(3)):not(:nth-child(8)),\n      .toolkit-overview-in-process > :not(:nth-child(2)):not(:nth-child(3)):not(:nth-child(10)),\n      .toolkit-overview-lobby > :not(:nth-child(2)):not(:nth-child(3)):not(:nth-child(7)),\n      .toolkit-overview-misc > :not(:nth-child(2)):not(:nth-child(3)):not(:nth-child(7)):not(:nth-child(11)):not(:nth-child(12)):not(:nth-child(13)),\n      .toolkit-overview-claim-tools > :not(:nth-child(2)):not(:nth-child(3)):not(:nth-child(6)),\n      .toolkit-overview-friend-tools > :not(:nth-child(2)):not(:nth-child(3)):not(:nth-child(6)),\n      .toolkit-overview-client > :not(:nth-child(2)):not(:nth-child(3)):not(:nth-child(9)):not(:nth-child(14)):not(:nth-child(15)):not(:nth-child(16)) { display: none; }\n    `}</style>
    <div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-bold">League 客户端工具箱</h2><p className="mt-1 text-xs text-cs2-text-muted">读取任务、奖励、活动、战利品与好友概况；账号写入默认开启，可随时关闭回到只读模式。</p></div><div className="flex gap-2"><button data-testid="league-account-actions-toggle" onClick={toggleAccountActions} disabled={busy} aria-pressed={Boolean(settings?.toolkit_account_actions_enabled)} className={`inline-flex items-center gap-2 rounded-xl border px-4 py-2.5 text-xs font-bold shadow-sm ${settings?.toolkit_account_actions_enabled?"border-emerald-400/50 bg-emerald-400/15 text-emerald-200":"border-rose-400/50 bg-rose-400/10 text-rose-200"}`}><span className={`h-2.5 w-2.5 rounded-full ${settings?.toolkit_account_actions_enabled?"bg-emerald-300":"bg-rose-300"}`}/><span>账号写入：{settings?.toolkit_account_actions_enabled?"已开启":"已关闭"}</span></button><button onClick={load} className="rounded-xl border border-cs2-border px-3 py-2 text-xs"><RefreshCw className={`mr-1 inline h-4 w-4 ${busy ? "animate-spin" : ""}`}/>刷新</button></div></div>
    <nav data-testid="league-toolkit-tabs" aria-label="客户端工具箱" className="flex max-w-full gap-1 overflow-x-auto rounded-xl border border-cs2-border bg-cs2-bg-elevated p-1">
      {TOOLKIT_TABS.map(([id, label, Icon]) => <button key={id} type="button" role="tab" aria-selected={toolkitTab === id} onClick={() => setToolkitTab(id)} className={`inline-flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-semibold transition-colors ${toolkitTab === id ? "bg-emerald-400/15 text-emerald-200" : "text-cs2-text-muted hover:text-cs2-text-primary"}`}><Icon className="h-3.5 w-3.5" />{label}</button>)}
    </nav>
    <section className="grid gap-3 md:grid-cols-4">{cards.map(([key,label,Icon])=><article key={key} className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><Icon className="mb-5 h-5 w-5 text-emerald-300"/><div className="text-2xl font-bold">{data?.counts?.[key] ?? "—"}</div><div className="mt-1 text-xs text-cs2-text-muted">{label}</div></article>)}</section>
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><h3 className="text-sm font-bold">进行中的任务</h3><div className="mt-3 grid gap-2">{(data?.missions||[]).slice(0,12).map((mission,index)=><div key={mission.id||index} className="rounded-xl border border-cs2-border-subtle px-3 py-2 text-xs"><b>{mission.title||mission.name||mission.id||"未命名任务"}</b><span className="ml-2 text-cs2-text-muted">{mission.status||mission.state||""}</span></div>)}{data&&!data.missions?.length&&<div className="text-xs text-cs2-text-muted">暂无可显示任务</div>}</div></section>
    <LeagueAccountTools mode={toolkitTab === "friend-tools" ? "friends" : toolkitTab === "claim-tools" ? "claims" : "all"} data={data} enabled={Boolean(settings?.toolkit_account_actions_enabled)} busy={busy} onBusyChange={setBusy} onRefresh={load} onError={onError} onOpenPlayer={onOpenPlayer}/>
    <LeagueAdvancedToolkit section={toolkitTab === "lobby" ? "lobby" : "misc"} enabled={Boolean(settings?.toolkit_account_actions_enabled)} busy={busy} onBusyChange={setBusy} onError={onError} onDryRunGame={onDryRunGame} onOpenPlayer={onOpenPlayer} streamerMode={streamerMode} useAliases={useAliases}/>
    <LeagueInGamePresetTools settings={settings} busy={busy} onSettingsUpdate={onSettingsUpdate} onBusyChange={setBusy} onError={onError} streamerMode={streamerMode} useAliases={useAliases}/>
    <LeagueAuxShortcutSettings settings={settings} busy={busy} onSettingsUpdate={onSettingsUpdate}/>
    <InProcessTools status={clientStatus} enabled={Boolean(settings?.toolkit_account_actions_enabled)} busy={busy} onBusyChange={setBusy} onError={onError} onRefresh={load}/>
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><h3 className="text-sm font-bold">聊天状态</h3><p className="mt-1 text-xs text-cs2-text-muted">与 LeagueAkari 相同，通过本机 LCU 修改；需先启用账号写入保护开关并点击“应用”。</p><div className="mt-4 grid gap-3 md:grid-cols-[220px_1fr_auto]"><select value={availability} onChange={(event)=>setAvailability(event.target.value)} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm"><option value="chat">在线</option><option value="mobile">手机在线</option><option value="away">离开</option><option value="offline">离线</option><option value="dnd">请勿打扰</option><option value="spectating">观战中</option><option value="online">游戏在线</option></select><input value={statusMessage} onChange={(event)=>setStatusMessage(event.target.value)} maxLength={500} placeholder="自定义状态消息" className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm outline-none"/><button onClick={applyPresence} disabled={busy||!data?.chat_presence||!settings?.toolkit_account_actions_enabled} className="rounded-xl bg-emerald-500 px-4 py-2.5 text-sm font-bold text-black disabled:opacity-40">应用</button></div></section>
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><div className="flex items-start gap-3"><Trophy className="mt-0.5 h-5 w-5 text-amber-300"/><div className="min-w-0 flex-1"><h3 className="text-sm font-bold">排位展示</h3><p className="mt-1 text-xs text-cs2-text-muted">只修改聊天名片上的展示，不改变真实段位；需先启用账号写入保护。自动登录恢复请在“自动游戏流程”中单独开启。</p><div className="mt-4 grid gap-3 md:grid-cols-[1.4fr_1fr_100px_auto]"><select value={rankedStatus.queue} onChange={(e)=>setRankedStatus({...rankedStatus,queue:e.target.value})} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm">{rankedQueues.map(([value,label])=><option key={value} value={value}>{label}</option>)}</select><select value={rankedStatus.tier} onChange={(e)=>setRankedStatus({...rankedStatus,tier:e.target.value})} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm">{rankedTiers.map((value)=><option key={value} value={value}>{value}</option>)}</select><select value={rankedStatus.division} disabled={["MASTER","GRANDMASTER","CHALLENGER"].includes(rankedStatus.tier)} onChange={(e)=>setRankedStatus({...rankedStatus,division:e.target.value})} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm disabled:opacity-40">{["I","II","III","IV"].map((value)=><option key={value} value={value}>{value}</option>)}</select><button onClick={applyRanked} disabled={busy||!settings?.toolkit_account_actions_enabled} className="rounded-xl bg-amber-400 px-4 py-2.5 text-sm font-bold text-black disabled:opacity-40">应用</button></div></div></div></section>
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><h3 className="text-sm font-bold">分析预设消息</h3><p className="mt-1 text-xs text-cs2-text-muted">生成近期表现、组排和打野画像草稿；房间/选人阶段走 LCU，游戏中只向验证过的前台 League 进程逐行输入。</p><div className="mt-3 flex flex-wrap gap-2"><select aria-label="分析预设目标" value={presetTarget} onChange={(event)=>setPresetTarget(event.target.value)} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-1.5 text-xs"><option value="all">双方全部玩家</option><option value="friendly">仅己方</option><option value="enemy">仅敌方</option></select><button onClick={()=>generatePreset("form")} disabled={busy} className="rounded-lg border border-cyan-400/25 bg-cyan-400/10 px-3 py-1.5 text-xs text-cyan-200 disabled:opacity-40">生成近期表现草稿</button><button onClick={()=>generatePreset("premade")} disabled={busy} className="rounded-lg border border-violet-400/25 bg-violet-400/10 px-3 py-1.5 text-xs text-violet-200 disabled:opacity-40">生成组排关系草稿</button><button onClick={()=>generatePreset("jungle")} disabled={busy} className="rounded-lg border border-amber-400/25 bg-amber-400/10 px-3 py-1.5 text-xs text-amber-200 disabled:opacity-40">生成打野画像草稿</button></div><textarea value={preset} onChange={(event)=>setPreset(event.target.value)} maxLength={3000} rows={4} placeholder="例如：大家好，祝游戏愉快" className="mt-3 w-full resize-y rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm outline-none"/><div className="mt-2 flex justify-end"><button onClick={sendPreset} disabled={busy||!preset.trim()||!settings?.toolkit_account_actions_enabled||!settings?.in_game_send_enabled} className="rounded-xl border border-emerald-400/30 bg-emerald-400/10 px-4 py-2 text-xs font-semibold text-emerald-300 disabled:opacity-40">发送到当前阶段</button></div></section>
    <section className="rounded-2xl border border-rose-400/25 bg-rose-400/[.04] p-4"><div className="flex items-center gap-3"><AlertTriangle className="h-5 w-5 text-rose-300"/><div className="min-w-0 flex-1"><h3 className="text-sm font-bold">游戏进程应急控制</h3><p className="mt-1 text-xs text-cs2-text-muted">仅当 League 游戏进程处于前台时可结束；用于卡死或无法退出，不会结束 LeagueClientUx。</p></div><button onClick={terminateGame} disabled={busy} className="inline-flex items-center gap-2 rounded-xl border border-rose-400/30 bg-rose-400/10 px-4 py-2 text-xs font-semibold text-rose-200 disabled:opacity-40"><Power className="h-4 w-4"/>结束前台游戏</button></div></section>
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><div className="flex items-center gap-3"><Boxes className="h-5 w-5 text-amber-300"/><div className="min-w-0 flex-1"><h3 className="text-sm font-bold">游戏设置文件保护</h3><p className="mt-1 text-xs text-cs2-text-muted">{settingsFile?`${settingsFile.file_name} 当前为${settingsFile.mode==="readonly"?"只读":"可写"}`:"连接客户端后可读取 PersistedSettings.json 属性"}；只改变文件属性，不修改文件内容。</p></div><button onClick={toggleSettingsFile} disabled={busy||!settingsFile||!settings?.toolkit_account_actions_enabled} title={!settings?.toolkit_account_actions_enabled?"请先启用账号写入保护":""} className="rounded-xl border border-amber-400/30 bg-amber-400/10 px-4 py-2 text-xs font-semibold text-amber-200 disabled:opacity-40">切换为{settingsFile?.mode==="readonly"?"可写":"只读"}</button></div></section>
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><div className="flex flex-wrap items-center gap-3"><Maximize2 className="h-5 w-5 text-cyan-300"/><div className="min-w-[240px] flex-1"><h3 className="text-sm font-bold">LeagueClientUx 窗口修复</h3><p className="mt-1 text-xs text-cs2-text-muted">按客户端当前缩放比例重新设置外层与 CEF 内容窗口并居中。{clientWindow?` 当前约 ${clientWindow.width} × ${clientWindow.height}，缩放 ${clientWindow.zoom??"未知"}`:"请先显示 League 客户端窗口"}</p></div><label className="flex items-center gap-1 text-xs">W<input type="number" min="640" max="3840" value={windowSize.baseWidth} onChange={(event)=>setWindowSize({...windowSize,baseWidth:event.target.value})} className="w-24 rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5"/></label><label className="flex items-center gap-1 text-xs">H<input type="number" min="360" max="2160" value={windowSize.baseHeight} onChange={(event)=>setWindowSize({...windowSize,baseHeight:event.target.value})} className="w-24 rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5"/></label><button onClick={resizeClient} disabled={busy} className="rounded-xl border border-cyan-400/30 bg-cyan-400/10 px-4 py-2 text-xs font-semibold text-cyan-200 disabled:opacity-40">调整并居中</button></div></section>
  </div>;
}
