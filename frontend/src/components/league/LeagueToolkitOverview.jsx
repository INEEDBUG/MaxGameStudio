import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { AlertTriangle, AppWindow, Boxes, Gift, Maximize2, Power, RefreshCw, Trophy, Users } from "lucide-react";
import { fetchLeagueClientWindow, fetchLeagueGameSettingsFile, fetchLeagueLabStatus, fetchLeagueToolkitOverview, resizeLeagueClientWindow, terminateLeagueGameClient, updateLeagueChatPresence, updateLeagueRankedStatus, updateLeagueGameSettingsFile } from "../../api/leagueLabApi";
const LeagueAccountToolsImpl = lazy(() => import("./LeagueAccountTools"));
const LeagueAdvancedToolkitImpl = lazy(() => import("./LeagueAdvancedToolkit"));
const LeagueAuxShortcutSettingsImpl = lazy(() => import("./LeagueAuxShortcutSettings"));

const rankedQueues = [["RANKED_SOLO_5x5","单双排位"],["RANKED_FLEX_SR","灵活排位"],["RANKED_TFT","云顶之弈"]];
const rankedTiers = ["IRON","BRONZE","SILVER","GOLD","PLATINUM","EMERALD","DIAMOND","MASTER","GRANDMASTER","CHALLENGER"];
const TOOLKIT_TABS = [
  ["client", "客户端", AppWindow],
  ["misc", "高级工具", Boxes],
  ["claim-tools", "领取奖励", Gift],
  ["friend-tools", "好友", Users],
];

const LEGACY_TOOLKIT_TAB_ALIASES = {
  "in-game-send": "client",
  "in-process": "client",
  lobby: "misc",
};

function normalizeToolkitTab(value) {
  const candidate = LEGACY_TOOLKIT_TAB_ALIASES[value] || value;
  return TOOLKIT_TABS.some(([id]) => id === candidate) ? candidate : "client";
}

function initialToolkitTab() {
  try {
    return normalizeToolkitTab(new URLSearchParams(window.location.search).get("toolkit"));
  } catch {
    return "client";
  }
}

function LazyToolkitPanel({ children }) {
  return <Suspense fallback={<div className="rounded-2xl border border-cs2-border-subtle bg-cs2-bg-elevated/60 p-4 text-xs text-cs2-text-muted">正在加载工具模块…</div>}>{children}</Suspense>;
}

export default function LeagueToolkitOverview({ onError, settings, onSettingsUpdate, onDryRunGame, onOpenPlayer, requestedTab = "", streamerMode = Boolean(settings?.streamer_mode_enabled), useAliases = Boolean(settings?.streamer_mode_use_aliases) }) {
  const [data, setData] = useState(null);
  const [busy, setBusy] = useState(false);
  const [availability,setAvailability]=useState("chat");
  const [statusMessage,setStatusMessage]=useState("");
  const [rankedStatus,setRankedStatus]=useState({queue:"RANKED_SOLO_5x5",tier:"CHALLENGER",division:"I"});
  const [settingsFile,setSettingsFile]=useState(null);
  const [clientWindow,setClientWindow]=useState(null);
  const normalizedRequestedTab = requestedTab ? normalizeToolkitTab(requestedTab) : "";
  const [toolkitTab, setToolkitTab] = useState(() => normalizedRequestedTab || initialToolkitTab());
  const [windowSize,setWindowSize]=useState({baseWidth:1280,baseHeight:720});
  const loadRequest = useRef(0);
  const disposed = useRef(false);
  useEffect(() => {
    if (normalizedRequestedTab) setToolkitTab((current) => current === normalizedRequestedTab ? current : normalizedRequestedTab);
  }, [normalizedRequestedTab]);
  const load = async () => { const request=++loadRequest.current;setBusy(true); try { const [next,status]=await Promise.all([fetchLeagueToolkitOverview(),fetchLeagueLabStatus()]);if(disposed.current||request!==loadRequest.current)return;setData(next);setAvailability(next?.chat_presence?.availability||"chat");setStatusMessage(next?.chat_presence?.statusMessage||"");setRankedStatus(status?.settings?.ranked_status||{queue:"RANKED_SOLO_5x5",tier:"CHALLENGER",division:"I"});try{const file=await fetchLeagueGameSettingsFile();if(!disposed.current&&request===loadRequest.current)setSettingsFile(file);}catch{if(!disposed.current&&request===loadRequest.current)setSettingsFile(null);}try{const nextWindow=await fetchLeagueClientWindow();if(!disposed.current&&request===loadRequest.current)setClientWindow(nextWindow);}catch{if(!disposed.current&&request===loadRequest.current)setClientWindow(null);} } catch (error) { if(!disposed.current&&request===loadRequest.current)onError(error?.response?.data?.detail || "工具箱读取失败"); } finally { if(!disposed.current&&request===loadRequest.current)setBusy(false); } };
  const applyPresence=async()=>{setBusy(true);try{const next=await updateLeagueChatPresence({availability,status_message:statusMessage});setData((current)=>({...current,chat_presence:next.chat_presence}));}catch(error){onError(error?.response?.data?.detail||"聊天状态应用失败");}finally{setBusy(false);}};
  const applyRanked=async()=>{setBusy(true);try{await updateLeagueRankedStatus(rankedStatus);}catch(error){onError(error?.response?.data?.detail||"排位展示应用失败");}finally{setBusy(false);}};
  const terminateGame=async()=>{const confirmation=window.prompt("只会结束当前前台的 League of Legends.exe，未保存的本局状态可能丢失。\n请输入“我确认结束游戏”继续：");if(confirmation!=="我确认结束游戏")return;setBusy(true);try{await terminateLeagueGameClient(confirmation);}catch(error){onError(error?.response?.data?.detail||"结束游戏进程失败");}finally{setBusy(false);}};
  const toggleSettingsFile=async()=>{if(!settings?.toolkit_account_actions_enabled){onError("请先在工具箱中启用账号写入保护");return;}const mode=settingsFile?.mode==="readonly"?"writable":"readonly";setBusy(true);try{setSettingsFile(await updateLeagueGameSettingsFile(mode));}catch(error){onError(error?.response?.data?.detail||"修改游戏设置文件属性失败");}finally{setBusy(false);}};
  const resizeClient=async()=>{if(!window.confirm(`将 League 客户端内容区调整为 ${windowSize.baseWidth} × ${windowSize.baseHeight} 的缩放尺寸并居中，确定继续吗？`))return;setBusy(true);try{setClientWindow(await resizeLeagueClientWindow(Number(windowSize.baseWidth),Number(windowSize.baseHeight)));}catch(error){onError(error?.response?.data?.detail||"调整 League 客户端窗口失败");}finally{setBusy(false);}};
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
        if (value) setToolkitTab(normalizeToolkitTab(value));
      } catch { /* ignore malformed browser history entries */ }
    };
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);
  return <div className={`space-y-4 toolkit-overview-${toolkitTab}`}>
    <div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-bold">League 客户端工具箱</h2><p className="mt-1 text-xs text-cs2-text-muted">读取任务、奖励、活动、战利品与好友概况；账号写入默认开启，可随时关闭回到只读模式。</p></div><div className="flex gap-2"><button data-testid="league-account-actions-toggle" onClick={toggleAccountActions} disabled={busy} aria-pressed={Boolean(settings?.toolkit_account_actions_enabled)} className={`inline-flex items-center gap-2 rounded-xl border px-4 py-2.5 text-xs font-bold shadow-sm ${settings?.toolkit_account_actions_enabled?"border-emerald-400/50 bg-emerald-400/15 text-emerald-200":"border-rose-400/50 bg-rose-400/10 text-rose-200"}`}><span className={`h-2.5 w-2.5 rounded-full ${settings?.toolkit_account_actions_enabled?"bg-emerald-300":"bg-rose-300"}`}/><span>账号写入：{settings?.toolkit_account_actions_enabled?"已开启":"已关闭"}</span></button><button onClick={load} className="rounded-xl border border-cs2-border px-3 py-2 text-xs"><RefreshCw className={`mr-1 inline h-4 w-4 ${busy ? "animate-spin" : ""}`}/>刷新</button></div></div>
    <nav data-testid="league-toolkit-tabs" aria-label="客户端工具箱" className="flex max-w-full gap-1 overflow-x-auto rounded-xl border border-cs2-border bg-cs2-bg-elevated p-1">
      {TOOLKIT_TABS.map(([id, label, Icon]) => <button key={id} type="button" role="tab" aria-selected={toolkitTab === id} onClick={() => setToolkitTab(id)} className={`inline-flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-semibold transition-colors ${toolkitTab === id ? "bg-emerald-400/15 text-emerald-200" : "text-cs2-text-muted hover:text-cs2-text-primary"}`}><Icon className="h-3.5 w-3.5" />{label}</button>)}
    </nav>
    {(toolkitTab === "claim-tools" || toolkitTab === "friend-tools") && <LazyToolkitPanel><LeagueAccountToolsImpl mode={toolkitTab === "friend-tools" ? "friends" : "claims"} data={data} enabled={Boolean(settings?.toolkit_account_actions_enabled)} busy={busy} onBusyChange={setBusy} onRefresh={load} onError={onError} onOpenPlayer={onOpenPlayer}/></LazyToolkitPanel>}
    {toolkitTab === "misc" && <LazyToolkitPanel><LeagueAdvancedToolkitImpl enabled={Boolean(settings?.toolkit_account_actions_enabled)} busy={busy} onBusyChange={setBusy} onError={onError} onDryRunGame={onDryRunGame} onOpenPlayer={onOpenPlayer} streamerMode={streamerMode} useAliases={useAliases}/></LazyToolkitPanel>}
    {toolkitTab === "misc" && <>
      <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><h3 className="text-sm font-bold">聊天状态</h3><p className="mt-1 text-xs text-cs2-text-muted">通过本机 LCU 修改；需先启用账号写入保护开关并点击“应用”。</p><div className="mt-4 grid gap-3 md:grid-cols-[220px_1fr_auto]"><select value={availability} onChange={(event)=>setAvailability(event.target.value)} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm"><option value="chat">在线</option><option value="mobile">手机在线</option><option value="away">离开</option><option value="offline">离线</option><option value="dnd">请勿打扰</option><option value="spectating">观战中</option><option value="online">游戏在线</option></select><input value={statusMessage} onChange={(event)=>setStatusMessage(event.target.value)} maxLength={500} placeholder="自定义状态消息" className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm outline-none"/><button onClick={applyPresence} disabled={busy||!data?.chat_presence||!settings?.toolkit_account_actions_enabled} className="rounded-xl bg-emerald-500 px-4 py-2.5 text-sm font-bold text-black disabled:opacity-40">应用</button></div></section>
      <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><div className="flex items-start gap-3"><Trophy className="mt-0.5 h-5 w-5 text-amber-300"/><div className="min-w-0 flex-1"><h3 className="text-sm font-bold">排位展示</h3><p className="mt-1 text-xs text-cs2-text-muted">只修改聊天名片上的展示，不改变真实段位；需先启用账号写入保护。</p><div className="mt-4 grid gap-3 md:grid-cols-[1.4fr_1fr_100px_auto]"><select value={rankedStatus.queue} onChange={(e)=>setRankedStatus({...rankedStatus,queue:e.target.value})} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm">{rankedQueues.map(([value,label])=><option key={value} value={value}>{label}</option>)}</select><select value={rankedStatus.tier} onChange={(e)=>setRankedStatus({...rankedStatus,tier:e.target.value})} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm">{rankedTiers.map((value)=><option key={value} value={value}>{value}</option>)}</select><select value={rankedStatus.division} disabled={["MASTER","GRANDMASTER","CHALLENGER"].includes(rankedStatus.tier)} onChange={(e)=>setRankedStatus({...rankedStatus,division:e.target.value})} className="rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm disabled:opacity-40">{["I","II","III","IV"].map((value)=><option key={value} value={value}>{value}</option>)}</select><button onClick={applyRanked} disabled={busy||!settings?.toolkit_account_actions_enabled} className="rounded-xl bg-amber-400 px-4 py-2.5 text-sm font-bold text-black disabled:opacity-40">应用</button></div></div></div></section>
    </>}
    {toolkitTab === "client" && <>
      <LazyToolkitPanel><LeagueAuxShortcutSettingsImpl settings={settings} busy={busy} onSettingsUpdate={onSettingsUpdate}/></LazyToolkitPanel>
      <section className="rounded-2xl border border-rose-400/25 bg-rose-400/[.04] p-4"><div className="flex items-center gap-3"><AlertTriangle className="h-5 w-5 text-rose-300"/><div className="min-w-0 flex-1"><h3 className="text-sm font-bold">游戏进程应急控制</h3><p className="mt-1 text-xs text-cs2-text-muted">仅当 League 游戏进程处于前台时可结束；用于卡死或无法退出，不会结束 LeagueClientUx。</p></div><button onClick={terminateGame} disabled={busy} className="inline-flex items-center gap-2 rounded-xl border border-rose-400/30 bg-rose-400/10 px-4 py-2 text-xs font-semibold text-rose-200 disabled:opacity-40"><Power className="h-4 w-4"/>结束前台游戏</button></div></section>
      <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><div className="flex items-center gap-3"><Boxes className="h-5 w-5 text-amber-300"/><div className="min-w-0 flex-1"><h3 className="text-sm font-bold">游戏设置文件保护</h3><p className="mt-1 text-xs text-cs2-text-muted">{settingsFile?`${settingsFile.file_name} 当前为${settingsFile.mode==="readonly"?"只读":"可写"}`:"连接客户端后可读取 PersistedSettings.json 属性"}；只改变文件属性，不修改文件内容。</p></div><button onClick={toggleSettingsFile} disabled={busy||!settingsFile||!settings?.toolkit_account_actions_enabled} title={!settings?.toolkit_account_actions_enabled?"请先启用账号写入保护":""} className="rounded-xl border border-amber-400/30 bg-amber-400/10 px-4 py-2 text-xs font-semibold text-amber-200 disabled:opacity-40">切换为{settingsFile?.mode==="readonly"?"可写":"只读"}</button></div></section>
      <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><div className="flex flex-wrap items-center gap-3"><Maximize2 className="h-5 w-5 text-cyan-300"/><div className="min-w-[240px] flex-1"><h3 className="text-sm font-bold">LeagueClientUx 窗口修复</h3><p className="mt-1 text-xs text-cs2-text-muted">按客户端当前缩放比例重新设置外层与 CEF 内容窗口并居中。{clientWindow?` 当前约 ${clientWindow.width} × ${clientWindow.height}，缩放 ${clientWindow.zoom??"未知"}`:"请先显示 League 客户端窗口"}</p></div><label className="flex items-center gap-1 text-xs">W<input type="number" min="640" max="3840" value={windowSize.baseWidth} onChange={(event)=>setWindowSize({...windowSize,baseWidth:event.target.value})} className="w-24 rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5"/></label><label className="flex items-center gap-1 text-xs">H<input type="number" min="360" max="2160" value={windowSize.baseHeight} onChange={(event)=>setWindowSize({...windowSize,baseHeight:event.target.value})} className="w-24 rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5"/></label><button onClick={resizeClient} disabled={busy} className="rounded-xl border border-cyan-400/30 bg-cyan-400/10 px-4 py-2 text-xs font-semibold text-cyan-200 disabled:opacity-40">调整并居中</button></div></section>
    </>}
  </div>;
}
