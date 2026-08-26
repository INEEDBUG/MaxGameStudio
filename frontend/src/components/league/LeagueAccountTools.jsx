import { useEffect, useMemo, useState } from "react";
import { Gift, ShieldAlert, Trash2, Users } from "lucide-react";
import {
  claimLeagueEventRewards,
  claimLeagueMissionReward,
  claimLeagueRewardGrant,
  deleteLeagueFriends,
  fetchLeagueFriendMetadata,
} from "../../api/leagueLabApi";

const CLAIM_PHRASE = "我确认领取";
const DELETE_PHRASE = "我确认删除";

function uniqueRewards(rewards = [], idKey) {
  const seen = new Set();
  return rewards.filter((reward) => {
    const id = String(reward?.[idKey] || "");
    if (!id || seen.has(id)) return false;
    seen.add(id);
    return true;
  });
}

function formatFriendDate(value, fallback) {
  if (value === null || value === undefined || value === "") return fallback;
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return fallback;
  const seconds = Math.max(0, Math.floor((Date.now() - parsed.getTime()) / 1000));
  const relative = seconds < 60
    ? "刚刚"
    : seconds < 3600
      ? `${Math.floor(seconds / 60)} 分钟前`
      : seconds < 86400
        ? `${Math.floor(seconds / 3600)} 小时前`
        : `${Math.floor(seconds / 86400)} 天前`;
  return `${parsed.toLocaleString("zh-CN", { hour12: false })}（${relative}）`;
}

function SelectionChip({ checked, disabled, onChange, title, subtitle, onTitleClick }) {
  return <label className={`flex cursor-pointer items-start gap-2 rounded-xl border px-3 py-2 text-xs transition-colors ${checked ? "border-emerald-400/40 bg-emerald-400/10" : "border-cs2-border-subtle bg-cs2-bg-input/40"} ${disabled ? "cursor-not-allowed opacity-40" : ""}`}>
    <input type="checkbox" checked={checked} disabled={disabled} onChange={(event)=>onChange(event.target.checked)} className="mt-0.5"/>
    <span className="min-w-0">{onTitleClick?<button type="button" onClick={(event)=>{event.preventDefault();event.stopPropagation();onTitleClick();}} className="block break-words text-left font-bold text-cyan-200 hover:underline">{title}</button>:<b className="block break-words">{title}</b>}{subtitle&&<span className="mt-0.5 block break-words text-cs2-text-muted">{subtitle}</span>}</span>
  </label>;
}

export default function LeagueAccountTools({ data, enabled, busy, onBusyChange, onRefresh, onError, onOpenPlayer, mode = "all" }) {
  const [missionChoices,setMissionChoices]=useState({});
  const [grantChoices,setGrantChoices]=useState({});
  const [selectedFriends,setSelectedFriends]=useState([]);
  const [friendQuery,setFriendQuery]=useState("");
  const [friendMetadata,setFriendMetadata]=useState({});

  const run = async (task) => {
    onBusyChange(true);
    try {
      await task();
      await onRefresh();
    } catch (error) {
      onError(error?.response?.data?.detail || error?.message || "账号操作失败");
    } finally {
      onBusyChange(false);
    }
  };
  const confirmPhrase = (phrase, action) => window.prompt(`${action}\n请输入“${phrase}”继续：`) === phrase;
  const toggleChoice = (setter, key, value, checked, maximum) => setter((current)=>{
    const selected = current[key] || [];
    const next = checked ? [...selected, value] : selected.filter((item)=>item!==value);
    if (checked && next.length > maximum) return current;
    return {...current,[key]:next};
  });
  const friends = useMemo(()=>{
    const query=friendQuery.trim().toLowerCase();
    return (data?.friends||[]).filter((friend)=>!query||`${friend.gameName||""}#${friend.gameTag||""}`.toLowerCase().includes(query));
  },[data?.friends,friendQuery]);
  const friendPuuidKey = useMemo(
    () => (data?.friends||[]).map((friend)=>String(friend.puuid||"")).filter(Boolean).sort().join("|"),
    [data?.friends],
  );
  useEffect(()=>{
    let cancelled=false;
    if(!friendPuuidKey){setFriendMetadata({});return()=>{cancelled=true;};}
    fetchLeagueFriendMetadata()
      .then((payload)=>{if(!cancelled)setFriendMetadata(payload?.friends||{});})
      .catch(()=>{if(!cancelled)setFriendMetadata({});});
    return()=>{cancelled=true;};
  },[friendPuuidKey]);

  const showClaims = mode === "all" || mode === "claims";
  const showFriends = mode === "all" || mode === "friends";
  return <div className="space-y-4">
    <section className={`rounded-2xl border p-4 ${enabled?"border-amber-400/30 bg-amber-400/[.06]":"border-cs2-border bg-cs2-bg-elevated"}`}>
      <div className="flex items-start gap-3"><ShieldAlert className={`mt-0.5 h-5 w-5 ${enabled?"text-emerald-300":"text-rose-300"}`}/><div><h3 className="text-sm font-bold">账号写入保护</h3><p className="mt-1 text-xs leading-5 text-cs2-text-muted">账号写入默认开启，但仍受上方总开关保护；关闭后所有写入按钮立即回到只读。敏感操作仍需明确勾选目标并输入确认短语，不会随机选择奖励。</p></div></div>
      </section>

    {showClaims && <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
      <div className="flex items-center gap-2"><Gift className="h-4 w-4 text-emerald-300"/><h3 className="text-sm font-bold">任务奖励</h3></div>
      <div className="mt-3 grid gap-3">{(data?.claimable_missions||[]).map((mission)=>{
        const rewards=uniqueRewards(mission.rewards,"rewardGroup"), selected=missionChoices[mission.id]||[];
        const strategy=mission.rewardStrategy||{}, min=Math.max(1,Number(strategy.selectMinGroupCount)||1), max=Math.max(min,Number(strategy.selectMaxGroupCount)||1);
        return <article key={mission.id} className="rounded-xl border border-cs2-border-subtle p-3"><div className="flex flex-wrap items-start justify-between gap-2"><div><b className="text-sm">{mission.title||mission.internalName||mission.id}</b><p className="mt-1 text-[11px] text-cs2-text-muted">选择 {min}–{max} 个奖励组</p></div><button disabled={!enabled||busy||selected.length<min||selected.length>max} onClick={()=>{if(confirmPhrase(CLAIM_PHRASE,`领取任务“${mission.title||mission.id}”所选奖励？`))run(()=>claimLeagueMissionReward(String(mission.id),selected,CLAIM_PHRASE));}} className="rounded-lg border border-emerald-400/30 bg-emerald-400/10 px-3 py-1.5 text-xs font-semibold text-emerald-200 disabled:opacity-40">领取所选</button></div><div className="mt-3 grid gap-2 md:grid-cols-2">{rewards.map((reward)=>{const id=String(reward.rewardGroup),checked=selected.includes(id);return <SelectionChip key={id} checked={checked} disabled={!enabled||(!checked&&selected.length>=max)} onChange={(value)=>toggleChoice(setMissionChoices,String(mission.id),id,value,max)} title={reward.description||reward.uniqueName||id} subtitle={`${reward.quantity||1} × ${reward.rewardType||"奖励"}`}/>;})}</div></article>;
      })}{!data?.claimable_missions?.length&&<p className="text-xs text-cs2-text-muted">当前没有处于 SELECT_REWARDS 状态的任务。</p>}</div>
     </section>}

    {showClaims && <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
      <div className="flex items-center gap-2"><Gift className="h-4 w-4 text-cyan-300"/><h3 className="text-sm font-bold">待选择奖励</h3></div>
      <div className="mt-3 grid gap-3">{(data?.claimable_rewards||[]).map((grant)=>{const info=grant.info||{},group=grant.rewardGroup||{},rewards=uniqueRewards(group.rewards,"id"),selected=grantChoices[info.id]||[];const strategy=group.selectionStrategyConfig||{},min=Math.max(1,Number(strategy.minSelectionsAllowed)||1),max=Math.max(min,Number(strategy.maxSelectionsAllowed)||1);return <article key={info.id} className="rounded-xl border border-cs2-border-subtle p-3"><div className="flex flex-wrap items-start justify-between gap-2"><div><b className="text-sm">{group.localizations?.title||group.id||info.id}</b><p className="mt-1 text-[11px] text-cs2-text-muted">明确选择 {min}–{max} 项；不使用随机选择</p></div><button disabled={!enabled||busy||selected.length<min||selected.length>max} onClick={()=>{if(confirmPhrase(CLAIM_PHRASE,`领取“${group.localizations?.title||group.id}”所选内容？`))run(()=>claimLeagueRewardGrant(String(info.id),String(group.id),selected,CLAIM_PHRASE));}} className="rounded-lg border border-cyan-400/30 bg-cyan-400/10 px-3 py-1.5 text-xs font-semibold text-cyan-200 disabled:opacity-40">领取所选</button></div><div className="mt-3 grid gap-2 md:grid-cols-2">{rewards.map((reward)=>{const id=String(reward.id),checked=selected.includes(id);return <SelectionChip key={id} checked={checked} disabled={!enabled||(!checked&&selected.length>=max)} onChange={(value)=>toggleChoice(setGrantChoices,String(info.id),id,value,max)} title={reward.localizations?.title||reward.itemId||id} subtitle={`${reward.quantity||1} × ${reward.itemType||"奖励"}`}/>;})}</div></article>;})}{!data?.claimable_rewards?.length&&<p className="text-xs text-cs2-text-muted">当前没有 PENDING_SELECTION 奖励。</p>}</div>
     </section>}

    {showClaims && <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4"><h3 className="text-sm font-bold">活动中心奖励</h3><div className="mt-3 grid gap-2">{(data?.claimable_events||[]).map((event)=><article key={event.eventId} className="flex flex-wrap items-center gap-3 rounded-xl border border-cs2-border-subtle px-3 py-2"><div className="min-w-0 flex-1"><b className="text-sm">{event.eventInfo?.eventName||event.eventId}</b><p className="mt-1 text-[11px] text-cs2-text-muted">{event.eventInfo?.unclaimedRewardCount||0} 个未领取 · {(event.reward_options||[]).map((row)=>row.rewardName).filter(Boolean).join("、")||"由客户端领取全部可用项"}</p></div><button disabled={!enabled||busy} onClick={()=>{if(confirmPhrase(CLAIM_PHRASE,`领取活动“${event.eventInfo?.eventName||event.eventId}”全部可用奖励？`))run(()=>claimLeagueEventRewards(String(event.eventId),CLAIM_PHRASE));}} className="rounded-lg border border-violet-400/30 bg-violet-400/10 px-3 py-1.5 text-xs font-semibold text-violet-200 disabled:opacity-40">领取全部</button></article>)}{!data?.claimable_events?.length&&<p className="text-xs text-cs2-text-muted">当前没有活动中心未领取奖励。</p>}</div></section>}

    {showFriends && <section className="rounded-2xl border border-rose-400/20 bg-cs2-bg-elevated p-4"><div className="flex flex-wrap items-center gap-2"><Users className="h-4 w-4 text-rose-300"/><h3 className="mr-auto text-sm font-bold">好友管理</h3><input value={friendQuery} onChange={(event)=>setFriendQuery(event.target.value)} placeholder="搜索 Riot ID" className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-1.5 text-xs outline-none"/><button disabled={!enabled||busy||!selectedFriends.length} onClick={()=>{if(confirmPhrase(DELETE_PHRASE,`永久删除所选 ${selectedFriends.length} 位好友？`))run(()=>deleteLeagueFriends(selectedFriends,DELETE_PHRASE));}} className="inline-flex items-center gap-1 rounded-lg border border-rose-400/30 bg-rose-400/10 px-3 py-1.5 text-xs font-semibold text-rose-200 disabled:opacity-40"><Trash2 className="h-3.5 w-3.5"/>删除所选（{selectedFriends.length}）</button></div><div className="mt-3 grid max-h-72 gap-2 overflow-y-auto pr-1 md:grid-cols-2">{friends.map((friend)=>{const id=String(friend.id||""),checked=selectedFriends.includes(id),metadata=friendMetadata[String(friend.puuid||"")]||{};return <SelectionChip key={id||friend.puuid} checked={checked} disabled={!enabled||!id} onChange={(value)=>setSelectedFriends((current)=>value?[...current,id]:current.filter((item)=>item!==id))} onTitleClick={friend.puuid&&onOpenPlayer?()=>onOpenPlayer(String(friend.puuid)):null} title={`${friend.gameName||friend.name||"未知好友"}${friend.gameTag?`#${friend.gameTag}`:""}`} subtitle={`${friend.availability||"offline"} · 上局 ${formatFriendDate(metadata.last_game_at,"从未进行对局")} · 好友自 ${formatFriendDate(metadata.friends_since,"未知")}`}/>;})}</div></section>}
  </div>;
}
