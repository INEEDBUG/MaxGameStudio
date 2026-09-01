import { useCallback, useEffect, useRef, useState } from "react";
import { Minus, Pin, PinOff, RefreshCw, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { acceptLeagueChampSelectTrade, cancelLeagueAutoAccept, cancelLeagueDodgeLoop, charityRerollLeagueChampion, declineLeagueChampSelectTrade, declineLeagueReadyCheck, fetchLeagueLabStatus, rerollLeagueChampion, runLeagueLabAction, saveLeagueLabSettings, selectLeagueChampionFromMini, selectLeagueChampionSkin, setLeagueAutoSelectTemporarilyDisabled, startLeagueDodgeLoop, stopLeagueMatchmaking, swapLeagueBenchChampion } from "../api/leagueLabApi";
import { getLeagueChampionIconUrl, getLeagueClientAssetUrl } from "../api/api";
import { maskLeagueName } from "../utils/leagueStreamerMode";

const PHASE_LABELS = {
  Lobby: "房间中",
  Matchmaking: "正在匹配",
  ReadyCheck: "对局已找到",
  ChampSelect: "英雄选择",
  InProgress: "游戏进行中",
  Reconnect: "等待重连",
  PreEndOfGame: "对局结算中",
  EndOfGame: "对局已结束",
  WaitingForStats: "等待战绩",
  None: "客户端空闲",
  Disconnected: "未连接",
};

const MATCHMAKING_STATUS_LABELS = {
  idle: "等待开始匹配",
  countdown: "即将开始匹配",
  searching: "搜索对局中",
  waiting_for_invitees: "等待邀请者回应",
  "waiting-for-invitees": "等待邀请者回应",
  waiting_for_penalty: "等待排队惩罚结束",
  "waiting-for-penalty": "等待排队惩罚结束",
  "waiting-for-penalty-time": "等待排队惩罚结束",
  insufficient_members: "等待更多房间成员",
  "insufficient-members": "等待更多房间成员",
  not_leader: "等待房主开始",
  "not-leader": "等待房主开始",
  lobby_unavailable: "暂时无法读取房间",
  "lobby-unavailable": "暂时无法读取房间",
  unsupported_lobby: "当前房间不支持自动匹配",
  "unsupported-lobby": "当前房间不支持自动匹配",
  cannot_start: "当前不能开始匹配",
  "cannot-start": "当前不能开始匹配",
  rematch_cancelled: "已按重排策略取消匹配",
  "rematch-cancelled": "已按重排策略取消匹配",
};

const ACCOUNT_ACTION_MESSAGE = "账号写入操作已关闭；请先在主窗口开启后再执行接受、秒退、重随、换位或皮肤操作。";

function getDisplayPhase(status) {
  if (!status?.connected) return "Disconnected";
  return status?.phase || "None";
}

function getRerollsRemaining(champSelect) {
  const value = Number(champSelect?.rerolls_remaining);
  return Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0;
}

// LeagueAkari's auxiliary bench derives this from the live LCU session.  A
// queue name (including ARAM/KIWI) is not evidence that the current client
// build exposes the reroll action.
function hasRerollEvidence(champSelect) {
  return champSelect?.allow_rerolling === true && getRerollsRemaining(champSelect) > 0;
}

function isSkinUnavailable(skin) {
  if (!skin || typeof skin !== "object") return true;
  if (skin.disabled === true || skin.owned === false || skin.unlocked === false) return true;
  if (skin.availability === false || skin.availability === "unavailable" || skin.availability === "locked") return true;
  if (skin.availability && typeof skin.availability === "object" && skin.availability.available === false) return true;
  return false;
}

function getSkinPreviewPath(skin, championId) {
  const path = skin?.preview_path || skin?.previewPath || skin?.splash_path || skin?.splashPath;
  return path || (championId ? getLeagueChampionIconUrl(championId) : "");
}

function getCountdownSeconds(countdown, now) {
  const dueAt = Number(countdown?.due_at);
  if (Number.isFinite(dueAt) && dueAt > 0) return Math.max(0, dueAt * 1000 - now) / 1000;
  if (countdown?.remaining_seconds === null || countdown?.remaining_seconds === undefined || countdown?.remaining_seconds === "") return null;
  const remaining = Number(countdown.remaining_seconds);
  return Number.isFinite(remaining) ? Math.max(0, remaining) : null;
}

function getActionProgress(actions) {
  if (!Array.isArray(actions) || actions.length === 0) return null;
  return {
    completed: actions.filter((action) => action?.completed).length,
    total: actions.length,
    active: actions.find((action) => action?.in_progress),
  };
}

function MiniSwitch({ label, checked, onChange, disabled = false, title }) {
  return <div className="flex items-center justify-between gap-3 py-1.5 text-xs text-zinc-300"><span>{label}</span><button type="button" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} title={title} onClick={() => onChange(!checked)} className={`relative h-5 w-9 appearance-none rounded-full p-0 transition-colors duration-150 active:scale-[.97] ${checked ? "bg-emerald-500" : "bg-zinc-700"} disabled:cursor-not-allowed disabled:opacity-40`}><span className={`absolute left-1 top-1 h-3 w-3 rounded-full bg-white transition-transform duration-150 ${checked ? "translate-x-4" : "translate-x-0"}`} /></button></div>;
}

// MIT-licensed source shape used so the auxiliary placeholder keeps the same
// visual weight and alignment as the reference Mini surface.
function AkariMiniLogo() {
  return <svg aria-hidden="true" viewBox="0 0 602.307 524.812" className="mini-akari-logo"><g transform="translate(-98.023 -143.992)"><path fill="currentColor" d="M499.426 249.63c74.9-3.8 144.4 38.1 180.5 103.8 19.4 35.5 23.6 74.4 18.8 113.8-30.2-72.6-80.4-122.8-152.6-151.5-37-14.7-76.2-21-113.6-15.3 78.8 15.8 150.7 47.8 213.3 106.2 20 18.7 36.7 41.6 51 70.3-1.8 10.5-4.1 20.6-7.4 30.8-41-60.9-102.1-86-165.6-88.6-37.6-1.5-74.6 4.1-100.1 16.3 89.6-8.9 218.5 17.6 264 77.5-9.2 30.3-27.1 59-48 82.5-34.6 38.9-88.2 67.6-140.6 70.5-51.8 2.9-102.1-17-140.5-51.3-38.8-35.2-62.3-84.3-65.4-136.6-3.1-56.4 21.9-116.6 59.2-158.1 37.8-41.9 90.6-67.2 146.9-70.3Z"/><path fill="currentColor" d="M243.252 143.992c30.8 18.4 72.9 91.8 94 155.1-29 40.3-53.3 89.7-66.5 149.6-47.7 18.1-112.4 17.1-170.4 11.2-5-6.4-.7-73.2 7.1-82.3 35.8 5.1 90.1 15.8 134.4 16.5-36.6-17.2-82.7-34-126.8-46 2.1-17 20.7-74.8 34.2-94.2 38.9 22.9 81 46.3 126.4 67.4-27.4-33.5-69.6-72.1-105.1-102.6 10.8-21.7 45.4-57 72.7-74.7Z"/></g></svg>;
}

// The LCU normally supplies the map artwork for the current queue.
// Keep a small local fallback so a missing optional asset never leaves a
// visually empty Lounge card while the client is still preparing its session.
function MiniMapIconFallback() {
  return <svg aria-hidden="true" viewBox="0 0 64 64" className="mini-map-icon mini-map-icon-fallback"><path d="M8 13.5 22 8l20 7 14-5.5v41L42 56 22 49 8 54.5v-41Z" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinejoin="round"/><path d="M22 8v41M42 15v41" fill="none" stroke="currentColor" strokeWidth="2" opacity=".8"/><path d="m15 24 10-4 10 4 11-4" fill="none" stroke="currentColor" strokeWidth="2" opacity=".55"/></svg>;
}

function MiniMapIcon({ src }) {
  // Associate failure with the exact URL. A mount effect that blindly resets
  // a boolean can race a very fast image error and restore the broken image.
  const [failedSrc, setFailedSrc] = useState("");

  if (!src || failedSrc === src) {
    return <span data-testid="mini-map-icon"><MiniMapIconFallback /></span>;
  }

  return <img
    data-testid="mini-map-icon"
    src={src}
    alt=""
    className="mini-map-icon"
    onLoad={(event) => {
      // A proxy can technically return HTTP 200 with an invalid/empty body;
      // naturalWidth keeps that case on the same local fallback path.
      if (event.currentTarget.naturalWidth === 0) {
        event.currentTarget.style.display = "none";
        setFailedSrc(src);
      }
    }}
    onError={(event) => {
      // Hide the native broken-image glyph before React replaces the node.
      event.currentTarget.style.display = "none";
      setFailedSrc(src);
    }}
  />;
}

const REMATCH_STRATEGY_LABELS = {
  never: "永不自动重排",
  "fixed-duration": "按固定时长重排",
  "estimated-duration": "按预计队列时长重排",
};

function MatchmakingControlCard({ status, phase, now, onUpdate }) {
  if (phase !== "Lobby" && phase !== "Matchmaking") return null;
  const settings = status?.settings || {};
  const masterEnabled = settings.automation_enabled === true;
  const featureEnabled = settings.auto_matchmaking_enabled === true;
  const effectiveEnabled = masterEnabled && featureEnabled;
  const dueAt = Number(status?.matchmaking_due_at);
  const startSeconds = Number.isFinite(dueAt) && dueAt > 0 ? Math.max(0, dueAt * 1000 - now) / 1000 : null;
  const search = status?.matchmaking_search;
  const strategy = settings.auto_matchmaking_rematch_strategy || "never";
  const elapsed = Number(search?.time_in_queue);
  const estimate = Number(search?.estimated_queue_time);
  const fixedDuration = Number(settings.auto_matchmaking_rematch_fixed_duration);
  const rematchLimit = strategy === "fixed-duration" && Number.isFinite(fixedDuration)
    ? fixedDuration
    : strategy === "estimated-duration" && Number.isFinite(estimate) ? estimate : null;
  const rematchSeconds = Number.isFinite(rematchLimit) && Number.isFinite(elapsed)
    ? Math.max(0, rematchLimit - elapsed) : null;
  // The feature remains configurable while the global automation
  // switch is off.  The master gate controls execution, not configuration.
  const controlDisabled = false;
  const inputClass = "w-20 rounded-lg border border-white/10 bg-zinc-900 px-2 py-1 text-right text-[11px] text-zinc-200 outline-none disabled:cursor-not-allowed disabled:opacity-40";
  return <section data-testid="mini-matchmaking-controls" className="mb-3 rounded-xl border border-cyan-400/20 bg-cyan-500/[.045] p-3">
    <div className="flex items-center justify-between gap-2"><div className="text-[10px] uppercase tracking-[.16em] text-cyan-300">自动匹配</div><span className={`text-[10px] font-semibold ${effectiveEnabled ? "text-emerald-200" : "text-amber-200"}`}>{effectiveEnabled ? "运行条件已启用" : masterEnabled ? "功能已关闭" : "总开关已关闭"}</span></div>
    <div className="mt-1 text-[10px] leading-4 text-zinc-500">只有自动化总开关和自动匹配同时开启时才会写入客户端；默认关闭。</div>
    <div className="mt-2 border-t border-white/10 pt-1">
      <MiniSwitch label="自动开始匹配" checked={featureEnabled} onChange={(value) => onUpdate({ auto_matchmaking_enabled: value })} />
      <MiniSwitch label="等待邀请中的成员" checked={settings.auto_matchmaking_wait_for_invitees !== false} onChange={(value) => onUpdate({ auto_matchmaking_wait_for_invitees: value })} />
    </div>
    <div className="mt-1 grid gap-2 text-[10px] text-zinc-400">
      <label className="flex items-center justify-between gap-2"><span>最低房间人数</span><input aria-label="Mini 自动匹配最低人数" type="number" min="1" max="99" disabled={controlDisabled} value={settings.auto_matchmaking_minimum_members ?? 1} onChange={(event) => onUpdate({ auto_matchmaking_minimum_members: Number(event.target.value) })} className={inputClass} /></label>
      <label className="flex items-center justify-between gap-2"><span>开始延迟（秒）</span><input aria-label="Mini 自动匹配开始延迟" type="number" min="0" step="0.5" disabled={controlDisabled} value={settings.auto_matchmaking_delay_seconds ?? 5} onChange={(event) => onUpdate({ auto_matchmaking_delay_seconds: Number(event.target.value) })} className={inputClass} /></label>
      <label className="flex items-center justify-between gap-2"><span>停止 / 重排策略</span><select aria-label="Mini 自动匹配重排策略" disabled={controlDisabled} value={strategy} onChange={(event) => onUpdate({ auto_matchmaking_rematch_strategy: event.target.value })} className="rounded-lg border border-white/10 bg-zinc-900 px-2 py-1 text-[11px] text-zinc-200 outline-none disabled:cursor-not-allowed disabled:opacity-40"><option value="never">永不</option><option value="fixed-duration">固定时长</option><option value="estimated-duration">预计队列时长</option></select></label>
      {strategy === "fixed-duration" && <label className="flex items-center justify-between gap-2"><span>固定时长（秒）</span><input aria-label="Mini 自动匹配固定时长" type="number" min="1" disabled={controlDisabled} value={settings.auto_matchmaking_rematch_fixed_duration ?? 2} onChange={(event) => onUpdate({ auto_matchmaking_rematch_fixed_duration: Number(event.target.value) })} className={inputClass} /></label>}
    </div>
    <div data-testid="mini-matchmaking-plan" className="mt-2 rounded-lg border border-white/10 bg-black/10 p-2 text-[10px] text-zinc-400">
      <div className="flex items-center justify-between gap-2"><span>当前策略</span><span className="font-semibold text-zinc-200">{REMATCH_STRATEGY_LABELS[strategy] || strategy}</span></div>
      {startSeconds != null && <div className="mt-1 flex items-center justify-between gap-2"><span>开始匹配倒计时</span><span className="font-semibold tabular-nums text-cyan-100">{formatObservedSeconds(startSeconds)}</span></div>}
      {strategy !== "never" && phase === "Matchmaking" && <div className="mt-1 flex items-center justify-between gap-2"><span>重排倒计时</span><span className="font-semibold tabular-nums text-cyan-100">{rematchSeconds == null ? "等待客户端数据" : formatObservedSeconds(rematchSeconds)}</span></div>}
      {["waiting_for_invitees", "waiting-for-invitees"].includes(status?.matchmaking_status) && <div className="mt-1 text-amber-200">仍有受邀成员未回应，自动匹配正在等待。</div>}
      {!effectiveEnabled && <div className="mt-1 text-zinc-500">当前仅展示状态，不会自动开始或重新匹配。</div>}
    </div>
  </section>;
}

function formatObservedSeconds(value) {
  const seconds = Number(value);
  return Number.isFinite(seconds) ? `${Math.max(0, seconds).toFixed(seconds >= 10 ? 0 : 1)} 秒` : "未知";
}

function getActionPlanRows(actionPlan, now) {
  if (!actionPlan || typeof actionPlan !== "object") return [];
  const rows = [];
  for (const [key, item] of [["accept_due", actionPlan.accept_due], ["phase_due", actionPlan.phase_due]]) {
    if (!item || typeof item !== "object") continue;
    rows.push({ key, label: item.label || key, item, seconds: getCountdownSeconds(item, now) });
  }
  for (const [index, item] of (Array.isArray(actionPlan.champion_due) ? actionPlan.champion_due : []).entries()) {
    if (!item || typeof item !== "object") continue;
    rows.push({ key: `champion_due_${item.action_id || index}`, label: item.label || "自动选择 / 禁用英雄", item, seconds: getCountdownSeconds(item, now) });
  }
  return rows;
}

const AUTO_SELECT_MOVE_LABELS = {
  "pick-intent": "预选英雄",
  "show-pick": "亮出英雄",
  "complete-pick": "锁定英雄",
  "show-ban": "亮出禁用",
  "complete-ban": "锁定禁用",
  vote: "投票",
  "show-subset-pick": "子集选人并锁定",
  "complete-subset-pick": "锁定子集英雄",
  "subset-bench-swap": "子集换位",
  "bench-swap": "备战席换位",
};

const AUTO_SELECT_MOVE_META = {
  "pick-intent": { expected: "expected_picks", actionability: "intent", kind: "pick" },
  "show-pick": { expected: "expected_picks", actionability: "show", kind: "pick" },
  "complete-pick": { expected: "expected_picks", actionability: "complete", kind: "pick" },
  "show-ban": { expected: "expected_bans", actionability: "show", kind: "ban" },
  "complete-ban": { expected: "expected_bans", actionability: "complete", kind: "ban" },
  vote: { expected: null, actionability: "vote", kind: "vote" },
  "show-subset-pick": { expected: "expected_picks", actionability: "subset_pick", kind: "pick" },
  "complete-subset-pick": { expected: "expected_picks", actionability: "subset_pick", kind: "pick" },
  "subset-bench-swap": { expected: "expected_swaps", actionability: "bench_swap", kind: "bench" },
  "bench-swap": { expected: "expected_swaps", actionability: "bench_swap", kind: "bench" },
};

const AUTO_SELECT_STATUS_LABELS = {
  unknown: "未知",
  "not-owned": "未拥有",
  unpickable: "当前不可选",
  banned: "已禁用",
  "pick-intented": "已被预选",
  picked: "已被选择",
  "subset-pickable": "子集可选",
  pickable: "可选择",
  unbannable: "当前不可禁用",
  bannable: "可禁用",
  unswappable: "不可换位",
  "subset-swappable": "子集可换位",
  "waiting-on-finalization": "等待最终化",
  swappable: "可换位",
};

const AUTO_SELECT_PLAN_META = [
  ["delayed_pick", "自动选人计划"],
  ["delayed_ban", "自动禁用计划"],
  ["delayed_bench_swap", "自动备战席换位"],
  ["delayed_trade", "自动处理换英雄"],
];

function getAutoSelectMoveLabel(move) {
  return AUTO_SELECT_MOVE_LABELS[move] || (move ? `未知动作 · ${move}` : "动作尚未确定");
}

function getAutoSelectStatusLabel(status) {
  return AUTO_SELECT_STATUS_LABELS[status] || (status ? `未知状态 · ${status}` : "未知");
}

function getAutoSelectRows(autoSelect, key) {
  return key && Array.isArray(autoSelect?.[key]) ? autoSelect[key].filter((row) => row && typeof row === "object") : [];
}

function getAutoSelectGate(autoSelect, status) {
  const config = autoSelect?.config;
  const master = typeof config?.master_enabled === "boolean"
    ? config.master_enabled
    : typeof status?.settings?.automation_enabled === "boolean" ? status.settings.automation_enabled : null;
  const feature = typeof config?.feature_enabled === "boolean"
    ? config.feature_enabled
    : typeof status?.settings?.auto_select_enabled === "boolean" ? status.settings.auto_select_enabled : null;
  const temporarilyDisabled = typeof autoSelect?.temporarily_disabled === "boolean"
    ? autoSelect.temporarily_disabled
    : typeof status?.auto_select_temporarily_disabled === "boolean" ? status.auto_select_temporarily_disabled : null;
  const enabled = typeof autoSelect?.enabled === "boolean" ? autoSelect.enabled : null;
  let label = "门控状态未返回 · 仅展示";
  let tone = "text-zinc-400";
  if (temporarilyDisabled === true) {
    label = "临时暂停自动选择";
    tone = "text-amber-200";
  } else if (master === false) {
    label = "自动化总开关已关闭";
    tone = "text-amber-200";
  } else if (feature === false || enabled === false) {
    label = "自动选择功能已关闭";
    tone = "text-amber-200";
  } else if (master === true && feature === true && enabled !== false) {
    label = "自动选择已启用";
    tone = "text-emerald-200";
  }
  return { label, tone, master, feature, temporarilyDisabled, enabled };
}

function getAutoSelectTaskTime(task, now) {
  if (!task || typeof task !== "object") return { seconds: null, progress: null };
  const seconds = getCountdownSeconds({
    due_at: task.due_at ?? task.finish_at ?? task.finishAt,
    remaining_seconds: task.remaining_seconds,
  }, now);
  const startAt = Number(task.start_at ?? task.startAt);
  const finishAt = Number(task.finish_at ?? task.finishAt);
  if (Number.isFinite(startAt) && Number.isFinite(finishAt) && finishAt > startAt) {
    const current = now / 1000;
    return {
      seconds,
      progress: Math.min(100, Math.max(0, ((current - startAt) / (finishAt - startAt)) * 100)),
    };
  }
  return { seconds, progress: null };
}

function getAutoSelectTaskTarget(task, key) {
  if (!task || typeof task !== "object") return "";
  const championId = task.champion_id ?? task.championId ?? task.requester_champion_id ?? task.requesterChampionId;
  if (championId === undefined || championId === null || championId === "") return "";
  const action = task.operation || task.action;
  if (key === "delayed_trade" && action) return ` · ${action === "accept" ? "接受" : action === "decline" ? "拒绝" : action} #${championId}`;
  return ` · #${championId}`;
}

function AutoSelectContextCard({ status, phase, now }) {
  const autoSelect = status?.auto_select;
  if (phase !== "ChampSelect" || !autoSelect || typeof autoSelect !== "object") return null;
  const move = autoSelect.move;
  const meta = AUTO_SELECT_MOVE_META[move];
  const gate = getAutoSelectGate(autoSelect, status);
  const expectedRows = getAutoSelectRows(autoSelect, meta?.expected);
  const actionability = autoSelect.actionability;
  const actionabilityValue = meta && actionability && typeof actionability[meta.actionability] === "boolean"
    ? actionability[meta.actionability]
    : null;
  const currentAction = autoSelect.current_action;
  const currentChampionId = currentAction?.champion_id || currentAction?.championId;
  const subset = autoSelect.subset;
  const subsetUnavailable = (move === "show-subset-pick" || move === "complete-subset-pick" || move === "subset-bench-swap")
    && subset && subset.available === false;
  const statusCounts = expectedRows.reduce((counts, row) => {
    const key = row.status || "unknown";
    counts[key] = (counts[key] || 0) + 1;
    return counts;
  }, {});
  const hasActionEvidence = Boolean(currentAction || move || expectedRows.length || subset || actionability);
  return <section data-testid="mini-auto-select-context" className="mb-3 rounded-xl border border-emerald-400/20 bg-emerald-500/[.045] p-3" aria-live="polite">
    <div className="flex items-center justify-between gap-2"><div className="text-[10px] uppercase tracking-[.16em] text-emerald-300">自动选人上下文</div><div className={`rounded-full border border-white/10 px-2 py-0.5 text-[10px] font-semibold ${gate.tone}`}>{gate.label}</div></div>
    <div className="mt-2 flex items-center justify-between gap-2"><span className="text-sm font-semibold text-white">{getAutoSelectMoveLabel(move)}</span>{currentAction && <span className="text-[10px] text-zinc-400">动作 #{String(currentAction.id ?? "未知")}</span>}</div>
    {!hasActionEvidence && <div className="mt-2 text-[10px] text-zinc-500">客户端尚未返回可证明的自动选人状态。</div>}
    {hasActionEvidence && <div className="mt-2 grid gap-2 text-[10px] text-zinc-400">
      {currentAction && <div className="flex items-center justify-between gap-2"><span>当前动作</span><span className="font-semibold text-zinc-200">{currentAction.type || "未知"} · {currentAction.completed ? "已完成" : currentAction.in_progress ? "进行中" : "等待"}{currentChampionId ? ` · 英雄 #${currentChampionId}` : ""}</span></div>}
      {meta?.kind === "vote" && <div className="rounded-lg border border-violet-400/20 bg-violet-400/[.06] p-2 text-violet-100">upstream 投票动作仅展示当前客户端状态，不自动提交投票。</div>}
      {subset && (meta?.kind === "pick" || meta?.kind === "bench") && <div className="flex items-center justify-between gap-2"><span>子集英雄池</span><span className={subsetUnavailable ? "font-semibold text-amber-200" : "font-semibold text-zinc-200"}>{subset.available === false ? "未返回" : `${Array.isArray(subset.ids) ? subset.ids.length : 0} 个`}</span></div>}
      {meta && <div className="flex items-center justify-between gap-2"><span>动作通道</span><span className={actionabilityValue === true ? "font-semibold text-emerald-200" : actionabilityValue === false ? "font-semibold text-amber-200" : "font-semibold text-zinc-500"}>{actionabilityValue === true ? "可操作" : actionabilityValue === false ? "当前不可操作" : "未返回 · 仅展示"}</span></div>}
      {expectedRows.length > 0 && <div data-testid={`mini-auto-expected-${meta?.expected || "unknown"}`} className="rounded-lg border border-white/10 bg-black/10 p-2"><div className="mb-1 flex items-center justify-between gap-2"><span>候选状态</span><span className="text-zinc-500">{expectedRows.length} 个</span></div><div className="flex flex-wrap gap-1">{expectedRows.slice(0, 8).map((row) => <span key={String(row.id)} className="inline-flex items-center gap-1 rounded-full border border-white/10 px-1.5 py-0.5 text-[10px] text-zinc-300"><span>#{row.id}</span><span className="text-emerald-200">{getAutoSelectStatusLabel(row.status)}</span></span>)}{expectedRows.length > 8 && <span className="rounded-full border border-white/10 px-1.5 py-0.5 text-[10px] text-zinc-500">+{expectedRows.length - 8}</span>}</div><div className="mt-1 text-[10px] text-zinc-500">{Object.entries(statusCounts).map(([key, count]) => `${getAutoSelectStatusLabel(key)} ${count}`).join(" · ")}</div></div>}
      {subsetUnavailable && <div className="text-amber-200">子集英雄池尚未返回，当前不推断可操作候选。</div>}
      {!move && <div className="text-zinc-500">动作未知；不会显示任何自动或手动操作。</div>}
      {move && !meta && <div className="text-amber-200">未识别的动作类型；仅保留原始状态。</div>}
    </div>}
  </section>;
}

function AutoSelectAutomationPlan({ status, phase, now }) {
  const autoSelect = status?.auto_select && typeof status.auto_select === "object" ? status.auto_select : {};
  const actionPlan = status?.action_plan && typeof status.action_plan === "object" ? status.action_plan : {};
  if (phase !== "ChampSelect") return null;
  const plans = AUTO_SELECT_PLAN_META.map(([key, label]) => ({ key, label, task: autoSelect[key] || actionPlan[key] }))
    .filter((entry) => entry.task && typeof entry.task === "object");
  if (!plans.length && Array.isArray(actionPlan.champion_due)) {
    plans.push(...actionPlan.champion_due.filter((task) => task && typeof task === "object").map((task, index) => ({
      key: `champion_due_${task.action_id || index}`,
      label: task.label || "自动选择 / 禁用英雄",
      task,
    })));
  }
  if (!plans.length) return null;
  const gate = getAutoSelectGate(autoSelect, status);
  return <section data-testid="mini-auto-select-plan" className="mb-3 rounded-xl border border-cyan-400/20 bg-cyan-500/[.045] p-3">
    <div className="flex items-center justify-between gap-2"><div className="text-[10px] uppercase tracking-[.16em] text-cyan-300">自动计划</div><span className={gate.tone}>{gate.temporarilyDisabled ? "已暂停" : "只读计划"}</span></div>
    <div className="mt-2 grid gap-2">{plans.map(({ key, label, task }) => { const timing = getAutoSelectTaskTime(task, now); return <div key={key} data-testid={`mini-auto-plan-${key}`} className="rounded-lg border border-white/10 bg-black/10 p-2"><div className="flex items-center justify-between gap-2 text-[10px]"><span className="font-semibold text-zinc-200">{label}{getAutoSelectTaskTarget(task, key)}</span><span className="tabular-nums text-cyan-100">{timing.seconds == null ? "倒计时不可用" : formatObservedSeconds(timing.seconds)}</span></div><div className="mt-1 flex items-center justify-between gap-2 text-[10px] text-zinc-500"><span>{task.move || task.operation || task.action || "等待执行"}{task.completed === true ? " · 完成动作" : ""}</span><span>{timing.progress == null ? "进度未返回" : `${Math.round(timing.progress)}%`}</span></div>{timing.progress != null && <div className="mt-1 h-1 overflow-hidden rounded-full bg-white/10"><div className="h-full rounded-full bg-cyan-400" style={{ width: `${timing.progress}%` }} /></div>}</div>; })}</div>
  </section>;
}

function PhaseContextCard({ status, phase, actionCountdown, actionSeconds, phaseSeconds, now }) {
  const actionProgress = phase === "ChampSelect" ? getActionProgress(status?.champ_select?.my_actions) : null;
  const actionProgressLabel = status?.settings?.auto_select_enabled ? "自动计划进度" : "客户端动作进度";
  const matchmakingStatus = MATCHMAKING_STATUS_LABELS[status?.matchmaking_status] || status?.matchmaking_status;
  const readyCheck = status?.ready_check;
  const matchmakingSearch = status?.matchmaking_search;
  const actionPlanRows = getActionPlanRows(status?.action_plan, now);
  const readyTimerSeconds = getCountdownSeconds(readyCheck?.timer, now);
  const hasActionCountdown = actionSeconds != null && actionCountdown;
  const hasPhaseTimer = phase === "ChampSelect" && phaseSeconds != null;
  const hasActionProgress = Boolean(actionProgress);
  const hasMatchmakingStatus = (phase === "Lobby" || phase === "Matchmaking") && Boolean(matchmakingStatus);
  const hasReadyCheck = phase === "ReadyCheck" && readyCheck && typeof readyCheck === "object";
  const hasMatchmakingSearch = (phase === "Lobby" || phase === "Matchmaking") && matchmakingSearch && typeof matchmakingSearch === "object";
  const hasActionPlan = actionPlanRows.length > 0;
  const hasDetails = hasActionCountdown || hasPhaseTimer || hasActionProgress || hasMatchmakingStatus || hasReadyCheck || hasMatchmakingSearch || hasActionPlan;
  let description = "等待本机 League 客户端返回当前阶段。";
  if (phase === "Lobby") description = "房间已连接；可以等待队友或开始匹配。";
  if (phase === "Matchmaking") description = "客户端正在搜索对局，Mini 会持续显示匹配状态。";
  if (phase === "ReadyCheck") description = "对局已找到，请在客户端接受窗口结束前处理。";
  if (phase === "ChampSelect") description = "正在进行英雄选择；下面只显示客户端已返回的计划与倒计时。";
  if (phase === "InProgress") description = "游戏进行中；Mini 将在有可证明的客户端状态时显示相关信息。";
  if (phase === "Reconnect") description = "客户端正在等待重连。";
  if (phase === "Disconnected") {
    if (status?.requires_elevation) description = "检测到 League 客户端，但读取 LCU 需要管理员权限。";
    else if (status?.last_error) description = `连接失败：${status.last_error}`;
    else if (status?.client_window_detected) description = "已检测到客户端窗口，正在等待 LCU 连接。";
    else if (!status) description = "正在读取客户端状态。";
    else description = "启动并登录 League 客户端后，Mini 会自动连接。";
  }
  if (phase === "None") description = "已连接 League 客户端，当前没有正在进行的对局流程。";
  return <section data-testid="mini-phase-context" className="mb-3 rounded-xl border border-sky-400/20 bg-sky-500/[.06] p-3" aria-live="polite">
    <div className="flex items-center justify-between gap-2"><div className="text-[10px] uppercase tracking-[.16em] text-sky-300">当前上下文</div><div className="rounded-full border border-sky-400/25 px-2 py-0.5 text-[10px] font-semibold text-sky-200">阶段 · {PHASE_LABELS[phase] || phase}</div></div>
    <p className="mt-1 text-[11px] leading-5 text-zinc-300">{description}</p>
    {hasDetails ? <div className="mt-2 grid gap-2 text-[10px] text-zinc-400">
      {hasMatchmakingStatus && <div className="flex items-center justify-between gap-2"><span>匹配状态</span><span className="font-semibold text-zinc-200">{matchmakingStatus}</span></div>}
      {hasActionCountdown && <div data-testid="mini-action-countdown" className="flex items-center justify-between gap-2"><span>{actionCountdown.label || "自动计划"}</span><span className="font-semibold tabular-nums text-emerald-200">{actionSeconds.toFixed(1)} 秒</span></div>}
      {hasPhaseTimer && <div data-testid="mini-phase-countdown" className="flex items-center justify-between gap-2"><span>{status?.champ_select?.timer_phase || "当前选择阶段"}</span><span className="font-semibold tabular-nums text-sky-200">{Math.ceil(phaseSeconds)} 秒</span></div>}
      {hasActionProgress && <div data-testid="mini-action-progress" className="flex items-center justify-between gap-2"><span>{actionProgressLabel}{actionProgress.active ? ` · ${actionProgress.active.type === "pick" ? "选择" : actionProgress.active.type === "ban" ? "禁用" : actionProgress.active.type}` : ""}</span><span className="font-semibold text-zinc-200">{actionProgress.completed}/{actionProgress.total} 已完成</span></div>}
      {hasReadyCheck && <div data-testid="mini-ready-check" className="rounded-lg border border-amber-400/20 bg-amber-400/[.06] p-2"><div className="flex items-center justify-between gap-2"><span>ReadyCheck · {readyCheck.state || "未知状态"}</span><span className="font-semibold text-amber-100">{readyCheck.player_response || "未响应"}</span></div>{readyTimerSeconds != null && <div className="mt-1 flex items-center justify-between gap-2"><span>剩余时间</span><span className="font-semibold tabular-nums text-amber-100">{formatObservedSeconds(readyTimerSeconds)}</span></div>}<div className="mt-1 text-[10px] text-zinc-500">{readyCheck.can_accept ? "可接受" : "当前不可接受"} · {readyCheck.can_decline ? "可拒绝" : "当前不可拒绝"}</div></div>}
      {hasMatchmakingSearch && <div data-testid="mini-matchmaking-search" className="rounded-lg border border-cyan-400/20 bg-cyan-400/[.05] p-2"><div className="flex items-center justify-between gap-2"><span>匹配搜索</span><span className="font-semibold text-cyan-100">{matchmakingSearch.search_state || (matchmakingSearch.is_currently_in_queue ? "搜索中" : "未搜索")}</span></div><div className="mt-1 grid grid-cols-2 gap-1 text-[10px]"><span>队列中：{matchmakingSearch.is_currently_in_queue ? "是" : "否"}</span>{matchmakingSearch.time_in_queue != null && <span>已等待：{formatObservedSeconds(matchmakingSearch.time_in_queue)}</span>}{matchmakingSearch.estimated_queue_time != null && <span>预计：{formatObservedSeconds(matchmakingSearch.estimated_queue_time)}</span>}{matchmakingSearch.queue_id != null && <span>队列 ID：{matchmakingSearch.queue_id}</span>}</div>{(matchmakingSearch.errors || []).length > 0 && <div className="mt-1 text-rose-200">{matchmakingSearch.errors.map((error, index) => <div key={`${error.code || "error"}-${index}`}>{error.message || error.code || "匹配错误"}</div>)}</div>}</div>}
      {hasActionPlan && <div data-testid="mini-action-plan" className="rounded-lg border border-emerald-400/20 bg-emerald-400/[.05] p-2"><div className="mb-1 font-semibold text-emerald-100">自动计划</div>{actionPlanRows.map((row) => <div key={row.key} className="flex items-center justify-between gap-2"><span>{row.label}</span><span className="font-semibold tabular-nums text-emerald-100">{row.seconds != null ? formatObservedSeconds(row.seconds) : "已暂停 / 等待状态"}</span></div>)}</div>}
    </div> : <div className="mt-2 text-[10px] text-zinc-500">客户端尚未返回可显示的阶段细节。</div>}
  </section>;
}

function LeagueMiniView({
  status,
  phase,
  now,
  pinned,
  message,
  canWrite,
  dodgeConfirmOpen,
  dodgeSubmitting,
  visibleSummonerName,
  readyCheck,
  matchmakingSearch,
  actionPlan,
  currentChampionId,
  team,
  bench,
  skinSelector,
  trades,
  respawn,
  actionCountdown,
  actionSeconds,
  phaseSeconds,
  compactDescription,
  setWindowPinned,
  load,
  minimizeWindow,
  closeWindow,
  update,
  accept,
  declineReady,
  cancelAutoAccept,
  stopMatchmaking,
  applyTradeAction,
  reroll,
  charityReroll,
  swapBench,
  selectChampionFromMini,
  selectSkin,
  startDodgeLoop,
  confirmDodgeLoop,
  cancelDodgeConfirmation,
  cancelDodgeLoop,
  setTemporarilyDisabled,
}) {
  const loungePhase = phase === "Lobby" || phase === "Matchmaking" || phase === "ReadyCheck";
  const mapIcon = status?.gameflow?.session?.map?.assets?.["game-select-icon-hover"]
    || status?.gameflow_session?.map?.assets?.["game-select-icon-hover"]
    || status?.map_icon
    || status?.map_icon_url;
  const mapIconUrl = getLeagueClientAssetUrl(mapIcon);
  const gameMode = status?.gameflow?.session?.gameData?.queue?.name
    || status?.gameflow_session?.gameData?.queue?.name
    || status?.queue_name
    || status?.game_mode;
  const mapName = status?.gameflow?.session?.map?.name
    || status?.gameflow_session?.map?.name
    || status?.map_name;
  const mapModeText = gameMode && mapName && gameMode !== mapName ? `${gameMode} · ${mapName}` : gameMode || mapName || "英雄联盟";
  const autoAcceptDelay = Number(status?.settings?.auto_accept_delay_seconds);
  const autoMatchDelay = Number(status?.settings?.auto_matchmaking_delay_seconds);
  const searchCountdown = Number(status?.matchmaking_due_at);
  const searchSeconds = Number.isFinite(searchCountdown) && searchCountdown > 0 ? Math.max(0, searchCountdown * 1000 - now) / 1000 : null;
  const readyTimerSeconds = getCountdownSeconds(readyCheck?.timer, now);
  const readyCanAccept = phase === "ReadyCheck" && readyCheck?.can_accept === true;
  const readyCanDecline = phase === "ReadyCheck" && readyCheck?.can_decline === true;
  const canStopMatchmaking = phase === "Matchmaking" && matchmakingSearch?.is_currently_in_queue === true;
  const currentChampionIcon = currentChampionId ? getLeagueChampionIconUrl(currentChampionId) : "";
  const rerollCount = getRerollsRemaining(status?.champ_select);
  const showRerollControls = hasRerollEvidence(status?.champ_select);
  const champActions = Array.isArray(status?.champ_select?.my_actions) ? status.champ_select.my_actions : [];
  const subsetChampions = Array.isArray(status?.champ_select?.subset_champion_ids)
    ? status.champ_select.subset_champion_ids.map(Number).filter((id) => Number.isFinite(id) && id > 0)
    : [];
  const benchChampions = Array.isArray(status?.champ_select?.bench_champions)
    ? status.champ_select.bench_champions.map(Number).filter((id) => Number.isFinite(id) && id > 0)
    : [];
  // LeagueAkari presents the server subset together with the bench during
  // BAN_PICK.  The first click locks the local pick; after that the same
  // icons remain available as bench swaps.  Keep the list evidence-driven and
  // remove the currently held champion from the clickable candidates.
  const miniCandidateChampions = Array.from(new Set(
    status?.champ_select?.allow_subset_champion_picks && status?.champ_select?.timer_phase === "BAN_PICK"
      ? [...subsetChampions, ...benchChampions]
      : benchChampions,
  )).filter((id) => id !== currentChampionId).slice(0, 10);
  const miniOpacityValue = Number(status?.settings?.mini_opacity);
  const miniOpacity = Number.isFinite(miniOpacityValue) ? Math.min(1, Math.max(0.4, miniOpacityValue)) : 1;

  const operationCard = <section data-testid="mini-lounge-operations" className="mini-card w-full">
    <MiniSwitch label={`自动接受${Number.isFinite(autoAcceptDelay) ? `（${autoAcceptDelay <= 0.05 ? "立即" : `${autoAcceptDelay.toFixed(1)} 秒`}）` : ""}`} checked={Boolean(status?.settings?.auto_accept_enabled)} onChange={(value) => update({ auto_accept_enabled: value })} />
    <details className="mini-operation-details">
      <summary className="mini-operation-row"><span>自动匹配{Number.isFinite(autoMatchDelay) ? `（${autoMatchDelay <= 0.05 ? "立即" : `${autoMatchDelay.toFixed(1)} 秒`}）` : ""}</span><span className="mini-chevron">⌄</span><MiniSwitch label="自动匹配（简洁）" checked={Boolean(status?.settings?.auto_matchmaking_enabled)} onChange={(value) => update({ auto_matchmaking_enabled: value })} /></summary>
      <div className="mini-popover-content">
        <MiniSwitch label="等待邀请中的成员" checked={status?.settings?.auto_matchmaking_wait_for_invitees !== false} onChange={(value) => update({ auto_matchmaking_wait_for_invitees: value })} />
        <label className="mini-input-row"><span>最低房间人数</span><input aria-label="Mini 简洁匹配最低人数" type="number" min="1" max="99" value={status?.settings?.auto_matchmaking_minimum_members ?? 1} onChange={(event) => update({ auto_matchmaking_minimum_members: Number(event.target.value) })} /></label>
        <label className="mini-input-row"><span>开始延迟（秒）</span><input aria-label="Mini 简洁匹配开始延迟" type="number" min="0" step="0.5" value={status?.settings?.auto_matchmaking_delay_seconds ?? 5} onChange={(event) => update({ auto_matchmaking_delay_seconds: Number(event.target.value) })} /></label>
      </div>
    </details>
  </section>;

  const champSelectOperations = <section data-testid="mini-dodge-loop" className="mini-card mini-operation-stack">
    {status?.connected && !canWrite && <div data-testid="mini-account-actions-disabled" className="mini-account-warning">账号写入操作已关闭；接受、秒退、重随、换位和皮肤操作已禁用。</div>}
    <div className="mini-operation-row"><span>秒退</span>{status?.dodge_loop?.active ? <button type="button" onClick={cancelDodgeLoop} className="mini-button mini-button-warning">取消循环</button> : dodgeConfirmOpen ? <span className="mini-button-row"><button type="button" disabled={dodgeSubmitting} onClick={confirmDodgeLoop} className="mini-button mini-button-warning disabled:cursor-not-allowed disabled:opacity-40">{dodgeSubmitting ? "正在执行" : "确认秒退"}</button><button type="button" disabled={dodgeSubmitting} onClick={cancelDodgeConfirmation} className="mini-button disabled:cursor-not-allowed disabled:opacity-40">取消</button></span> : <button type="button" disabled={!canWrite || dodgeSubmitting} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : "点击后需要再次确认"} onClick={startDodgeLoop} className="mini-button mini-button-primary disabled:cursor-not-allowed disabled:opacity-40">立即秒退</button>}</div>
    <div className="mini-operation-row"><span>临时取消自动选择 / 禁用</span><MiniSwitch label="临时取消自动选择 / 禁用" checked={Boolean(status?.auto_select_temporarily_disabled)} onChange={setTemporarilyDisabled} /></div>
    {status?.dodge_loop?.active && <div data-testid="mini-dodge-loop-status" className="mini-muted">正在运行 · 已发送 {Number(status.dodge_loop.attempts || 0)} 次</div>}
    {!status?.dodge_loop?.active && Number(status?.dodge_loop?.attempts || 0) > 0 && <div data-testid="mini-dodge-loop-status" className="mini-muted">上次发送 {Number(status.dodge_loop.attempts || 0)} 次 · {status.dodge_loop.stop_reason || "已停止"}</div>}
  </section>;

  const isLivePhase = ["GameStart", "InProgress", "Reconnect", "PreEndOfGame", "EndOfGame", "WaitingForStats"].includes(phase);
  const hasMapContext = Boolean(gameMode || mapName);
  const placeholderLabel = isLivePhase && hasMapContext ? mapModeText : phase === "InProgress" ? "游戏进行中" : "当前没有进行中的活动";

  return <div className="league-mini-shell">
    <div data-tauri-drag-region className="mini-titlebar"><span data-tauri-drag-region>MaxGameStudio Mini</span><span className="mini-titlebar-actions">
      <button type="button" aria-label={pinned ? "取消置顶" : "窗口置顶"} onClick={setWindowPinned} className={`mini-titlebar-button ${pinned ? "is-active" : ""}`}>{pinned ? <Pin className="h-3.5 w-3.5" /> : <PinOff className="h-3.5 w-3.5" />}</button>
      <button type="button" aria-label="最小化 Mini" onClick={minimizeWindow} className="mini-titlebar-button"><Minus className="h-3.5 w-3.5" /></button>
      <button type="button" aria-label="关闭 Mini" onClick={closeWindow} className="mini-titlebar-button mini-titlebar-close"><X className="h-3.5 w-3.5" /></button>
    </span></div>
    <div className={`mini-content ${phase === "ChampSelect" ? "mini-content-scroll" : ""}`}>
      <div className="mini-opacity-row">
        <label htmlFor="mini-opacity-slider">Mini 透明度 <span>{Math.round(miniOpacity * 100)}%</span></label>
        <input id="mini-opacity-slider" aria-label="Mini 不透明度" type="range" min="0.4" max="1" step="0.05" value={miniOpacity} onChange={(event) => update({ mini_opacity: Number(event.target.value) })} />
      </div>
      {loungePhase && <section data-testid="mini-lounge-view" className="mini-lounge-view">
        <div className="mini-lounge-center">
          <MiniMapIcon src={mapIconUrl} />
          {phase === "ChampSelect" && <div className="mini-primary-text">英雄选择</div>}
          {phase === "ReadyCheck" ? <>
            {actionPlan?.accept_due && <><div className="mini-primary-text">{actionPlan.accept_due.label || "即将自动接受"}</div><button type="button" onClick={cancelAutoAccept} className="mini-button mini-button-primary">取消自动接受</button><button type="button" disabled={!canWrite || !readyCanDecline} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : undefined} onClick={declineReady} className="mini-button mini-button-warning disabled:cursor-not-allowed disabled:opacity-40">拒绝对局</button></>}
            {!actionPlan?.accept_due && readyCheck?.player_response === "Accepted" && <><div className="mini-primary-text">已接受对局</div><div className="mini-secondary-text">等待其他玩家完成确认</div><button type="button" disabled={!canWrite || !readyCanDecline} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : undefined} onClick={declineReady} className="mini-button mini-button-warning disabled:cursor-not-allowed disabled:opacity-40">拒绝对局</button></>}
            {!actionPlan?.accept_due && readyCheck?.player_response === "Declined" && <><div className="mini-primary-text">已拒绝对局</div><div className="mini-secondary-text">可以等待下一次匹配</div><button type="button" disabled={!canWrite || !readyCanAccept} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : undefined} onClick={accept} className="mini-button mini-button-primary disabled:cursor-not-allowed disabled:opacity-40">立即接受</button></>}
            {!actionPlan?.accept_due && (!readyCheck?.player_response || readyCheck?.player_response === "None") && <><div className="mini-primary-text">对局已找到</div>{readyTimerSeconds != null && <div className="mini-secondary-text tabular-nums">剩余 {formatObservedSeconds(readyTimerSeconds)}</div>}<div className="mini-button-row"><button type="button" disabled={!canWrite || !readyCanAccept} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : !readyCanAccept ? "ReadyCheck 尚未返回可接受状态。" : undefined} onClick={accept} className="mini-button mini-button-primary disabled:cursor-not-allowed disabled:opacity-40">立即接受</button><button type="button" disabled={!canWrite || !readyCanDecline} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : undefined} onClick={declineReady} className="mini-button mini-button-warning disabled:cursor-not-allowed disabled:opacity-40">拒绝对局</button></div></>}
          </> : phase === "Matchmaking" ? <>
            {searchSeconds != null ? <><div className="mini-primary-text">{formatObservedSeconds(searchSeconds)} 后开始匹配</div><button type="button" onClick={() => update({ auto_matchmaking_enabled: false })} className="mini-button mini-button-primary">取消</button></> : <><div className="mini-primary-text">正在搜索对局</div>{matchmakingSearch && <div className="mini-secondary-text tabular-nums">{matchmakingSearch.time_in_queue != null ? `${formatObservedSeconds(matchmakingSearch.time_in_queue)} / ${formatObservedSeconds(matchmakingSearch.estimated_queue_time)}` : MATCHMAKING_STATUS_LABELS[status?.matchmaking_status] || "搜索中"}</div>}<button type="button" disabled={!canWrite || !canStopMatchmaking} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : undefined} onClick={stopMatchmaking} className="mini-button mini-button-warning disabled:cursor-not-allowed disabled:opacity-40">停止匹配</button></>}
           </> : <><div className="mini-primary-text" title={mapModeText}>{mapModeText}</div>{status?.matchmaking_status && <div className="mini-secondary-text">{MATCHMAKING_STATUS_LABELS[status.matchmaking_status] || status.matchmaking_status}</div>}</>}
        </div>
        {operationCard}
      </section>}
      {phase === "ChampSelect" && <section data-testid="mini-champ-select-view" className="mini-champ-select-view">
         {status?.champ_select?.bench_enabled && <section className="mini-card mini-bench-card"><div className="mini-bench-main">{currentChampionIcon ? <img src={currentChampionIcon} alt={String(currentChampionId)} className="mini-current-champion" /> : <div className="mini-current-champion mini-current-placeholder">?</div>}{showRerollControls && <div data-testid="mini-reroll-controls" className="mini-bench-buttons"><button disabled={!canWrite} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : undefined} onClick={reroll} className="mini-button mini-button-primary">重随 {rerollCount}</button><button data-testid="mini-charity-reroll" disabled={!canWrite || !currentChampionId} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : !currentChampionId ? "当前没有可记录的原英雄" : "需确认；仅在新会话证据允许时换回"} onClick={charityReroll} className="mini-button">慈善</button></div>}</div><div className="mini-bench-divider" /><div className="mini-bench-grid">{miniCandidateChampions.map((id) => <button key={id} type="button" data-testid={`mini-bench-champion-${id}`} aria-label={`选择英雄 ${id}`} disabled={!canWrite} title={!canWrite ? ACCOUNT_ACTION_MESSAGE : `点击选择或换取英雄 ${id}`} onClick={() => selectChampionFromMini(id)} className="mini-bench-champion disabled:cursor-not-allowed disabled:grayscale disabled:opacity-40"><img src={getLeagueChampionIconUrl(id)} alt={String(id)} /></button>)}{Array.from({ length: Math.max(10 - miniCandidateChampions.length, 0) }).map((_, index) => <div key={`empty-${index}`} className="mini-bench-champion mini-bench-empty" />)}</div></section>}
         {champActions.length > 0 && <section className="mini-card mini-actions-card"><div className="mini-timeline">{champActions.map((action, index) => <div key={action.id || `${action.type}-${index}`} className={`mini-timeline-item ${action.completed ? "is-complete" : action.in_progress ? "is-active" : ""}`}><span className="mini-timeline-dot" /><div className="mini-timeline-body"><div className="mini-timeline-title"><span>{action.type === "pick" ? "选择英雄" : action.type === "ban" ? "禁用英雄" : action.type === "vote" ? "投票" : action.type}</span>{action.in_progress ? "（进行中）" : action.completed ? "（已完成）" : ""}</div>{action.completed && action.champion_id ? <div className="mini-timeline-result"><img src={getLeagueChampionIconUrl(action.champion_id)} alt="" />{action.type === "pick" ? "已选择" : action.type === "ban" ? "已禁用" : "已完成"}</div> : null}</div></div>)}</div></section>}
         <AutoSelectAutomationPlan status={status} phase={phase} now={now} />
         {skinSelector.available && Array.isArray(skinSelector.skins) && skinSelector.skins.length > 0 && <section data-testid="mini-skin-selector" className="mini-card"><select aria-label="Mini 皮肤选择" value={skinSelector.selected_skin_id || ""} disabled={!canWrite || skinSelector.disabled === true} onChange={(event) => selectSkin(event.target.value)} className="mini-native-select"><option value="">选择皮肤（{skinSelector.skins.length}）</option>{skinSelector.skins.map((skin) => <option key={skin.id} value={skin.id} disabled={isSkinUnavailable(skin)}>{skin.name || `皮肤 ${skin.id}`}{skin.is_chroma ? " · 炫彩" : ""}</option>)}</select></section>}
         {champSelectOperations}
       </section>}
        {!loungePhase && phase !== "ChampSelect" && <div data-testid="mini-placeholder" className="mini-placeholder" aria-label="Placeholder"><AkariMiniLogo /><span className="mini-placeholder-label">{placeholderLabel}</span>{isLivePhase && hasMapContext && <span className="mini-placeholder-subtitle">{PHASE_LABELS[phase] || phase}</span>}</div>}
       {message && (loungePhase || phase === "ChampSelect") ? <div className="mini-message">{message}</div> : null}
    </div>
  </div>;
}

export default function LeagueMiniPanel() {
  const [status, setStatus] = useState(null);
  const [message, setMessage] = useState("");
  const [pinned, setPinned] = useState(true);
  const [now, setNow] = useState(Date.now());
  const [dodgeConfirmOpen, setDodgeConfirmOpen] = useState(false);
  const [dodgeSubmitting, setDodgeSubmitting] = useState(false);
  const pinMutationRef = useRef(0);
  const pendingPinnedRef = useRef(null);
  const load = useCallback(async () => {
    try {
      const next = await fetchLeagueLabStatus();
      setStatus(next);
      return next;
    } catch (error) {
      setStatus(null);
      setMessage(error?.response?.data?.detail || error?.message || "无法读取英雄联盟客户端状态。");
      return null;
    }
  }, []);
  useEffect(() => { load(); const id = setInterval(load, 1500); return () => clearInterval(id); }, [load]);
  useEffect(() => { const id = setInterval(() => setNow(Date.now()), 100); return () => clearInterval(id); }, []);
  useEffect(() => {
    document.documentElement.style.opacity = String(status?.settings?.mini_opacity ?? 1);
    return () => { document.documentElement.style.opacity = "1"; };
  }, [status?.settings?.mini_opacity]);
  useEffect(() => {
    if (typeof status?.settings?.mini_pinned !== "boolean") return;
    const next = status.settings.mini_pinned;
    if (pendingPinnedRef.current !== null && pendingPinnedRef.current !== next) return;
    if (pendingPinnedRef.current === next) pendingPinnedRef.current = null;
    setPinned(next);
    invoke("set_league_window_pinned", { kind: "mini", pinned: next }).catch(() => {});
  }, [status?.settings?.mini_pinned]);
  const applyStatusResult = useCallback(async (result, successMessage = "") => {
    if (result && typeof result === "object") {
      setStatus(result);
      if (successMessage) setMessage(successMessage);
      return result;
    }
    if (successMessage) setMessage(`${successMessage} 正在刷新状态。`);
    await load();
    return null;
  }, [load]);
  const update = async (patch) => {
    const currentSettings = status?.settings || {};
    const nextSettings = { ...currentSettings, ...patch };
    // LeagueAkari treats the global automation switch as the execution gate,
    // while each child feature remains independently configurable.  Mini is a
    // compact entry point, so turning on either supported automation must also
    // open that gate; do not turn on sibling features and do not close the gate
    // when one child is switched off.
    if (
      (patch.auto_accept_enabled === true || patch.auto_matchmaking_enabled === true)
      && currentSettings.automation_enabled !== true
    ) {
      nextSettings.automation_enabled = true;
    }
    try { await applyStatusResult(await saveLeagueLabSettings(nextSettings)); }
    catch (error) { setMessage(error?.response?.data?.detail || "设置更新失败"); }
  };
  const team = status?.champ_select?.my_team || [];
  const bench = status?.champ_select?.bench_champions || [];
  const currentChampionId = Number(status?.champ_select?.current_champion_id || 0);
  const respawn = status?.respawn_timer || {};
  const skinSelector = status?.settings?.mini_show_skin_selector === false ? {} : (status?.champ_select?.skin_selector || {});
  const actionCountdown = status?.action_countdown;
  const actionSeconds = getCountdownSeconds(actionCountdown, now);
  const phaseDeadline = status?.champ_select?.timer_deadline_at;
  const phaseSeconds = Number.isFinite(Number(phaseDeadline)) && Number(phaseDeadline) > 0 ? Math.max(0, Number(phaseDeadline) * 1000 - now) / 1000 : null;
  const streamerMode = Boolean(status?.settings?.streamer_mode_enabled);
  const canWrite = Boolean(status?.settings?.toolkit_account_actions_enabled);
  const phase = getDisplayPhase(status);
  useEffect(() => {
    if (phase !== "ChampSelect") setDodgeConfirmOpen(false);
  }, [phase]);
  const readyCheck = status?.ready_check;
  const matchmakingSearch = status?.matchmaking_search;
  const actionPlan = status?.action_plan;
  const readyCanAccept = phase === "ReadyCheck" && readyCheck?.can_accept === true;
  const readyCanDecline = phase === "ReadyCheck" && readyCheck?.can_decline === true;
  const canCancelAutoAccept = phase === "ReadyCheck" && Boolean(actionPlan?.accept_due);
  const canStopMatchmaking = phase === "Matchmaking" && matchmakingSearch?.is_currently_in_queue === true;
  const trades = Array.isArray(status?.champ_select?.trades) ? status.champ_select.trades : [];
  const visibleSummonerName = streamerMode ? maskLeagueName(status?.summoner_name, 0, status?.settings?.streamer_mode_use_aliases, status?.current_summoner?.puuid) : status?.summoner_name;
  const setWindowPinned = async () => {
    const next = !pinned;
    const mutation = pinMutationRef.current + 1;
    pinMutationRef.current = mutation;
    pendingPinnedRef.current = next;
    setPinned(next);
    try {
      const result = await saveLeagueLabSettings({ ...(status?.settings || {}), mini_pinned: next });
      if (mutation !== pinMutationRef.current) return;
      if (result && typeof result === "object" && typeof result.settings?.mini_pinned === "boolean") {
        const persisted = result.settings.mini_pinned;
        pendingPinnedRef.current = persisted;
        setPinned(persisted);
        await getCurrentWindow().setAlwaysOnTop(persisted);
        pendingPinnedRef.current = null;
        setStatus(result);
        return;
      }
      pendingPinnedRef.current = null;
      await getCurrentWindow().setAlwaysOnTop(next);
    } catch (error) {
      if (mutation === pinMutationRef.current) {
        pendingPinnedRef.current = null;
        setPinned(!next);
        await getCurrentWindow().setAlwaysOnTop(!next).catch(() => {});
      }
      setMessage(error?.message || "窗口置顶设置失败");
    }
  };
  const minimizeWindow = () => getCurrentWindow().minimize();
  const closeWindow = () => getCurrentWindow().close();
  const requireAccountActions = () => {
    if (!canWrite) { setMessage("账号写入操作已关闭；请先在主窗口开启后再执行此操作。"); return false; }
    return true;
  };
  const applyAccountAction = async (action, successMessage) => {
    if (!requireAccountActions()) return;
    try { await applyStatusResult(await runLeagueLabAction(action), successMessage); }
    catch (error) { setMessage(error?.response?.data?.detail || `${action} 操作失败`); }
  };
  const accept = async () => {
    if (!readyCanAccept) { setMessage("当前 ReadyCheck 不允许接受或状态证据尚未返回。"); return; }
    await applyAccountAction("accept", "已发送接受请求。");
  };
  const declineReady = async () => {
    if (!readyCanDecline) { setMessage("当前 ReadyCheck 不允许拒绝或状态证据尚未返回。"); return; }
    if (!requireAccountActions()) return;
    try { await applyStatusResult(await declineLeagueReadyCheck(), "已发送拒绝对局请求。"); }
    catch (error) { setMessage(error?.response?.data?.detail || error?.message || "拒绝对局失败"); }
  };
  const cancelAutoAccept = async () => {
    if (!canCancelAutoAccept) { setMessage("当前没有可取消的自动接受计划。"); return; }
    try { await applyStatusResult(await cancelLeagueAutoAccept(), "已取消本次自动接受。"); }
    catch (error) { setMessage(error?.response?.data?.detail || error?.message || "取消自动接受失败"); }
  };
  const stopMatchmaking = async () => {
    if (!canStopMatchmaking) { setMessage("当前没有可停止的匹配搜索。"); return; }
    if (!requireAccountActions()) return;
    try { await applyStatusResult(await stopLeagueMatchmaking(), "已发送停止匹配请求。"); }
    catch (error) { setMessage(error?.response?.data?.detail || error?.message || "停止匹配失败"); }
  };
  const applyTradeAction = async (trade, operation) => {
    if (!trade?.actionable || (operation === "accept" ? trade.can_accept !== true : trade.can_decline !== true)) {
      setMessage("该换英雄请求当前不可操作。");
      return;
    }
    if (!requireAccountActions()) return;
    try {
      const result = operation === "accept" ? await acceptLeagueChampSelectTrade(trade.id) : await declineLeagueChampSelectTrade(trade.id);
      await applyStatusResult(result, operation === "accept" ? "已接受换英雄请求。" : "已拒绝换英雄请求。");
    } catch (error) { setMessage(error?.response?.data?.detail || error?.message || "换英雄请求处理失败"); }
  };
  const reroll = async () => {
    if (!requireAccountActions()) return;
    try { await applyStatusResult(await rerollLeagueChampion(), "重随请求已发送。"); }
    catch (error) { setMessage(error?.response?.data?.detail || "重随失败"); }
  };
  const charityReroll = async () => {
    if (!requireAccountActions()) return;
    const confirmation = window.prompt("慈善重随会消耗一次重随，并在客户端证据允许时换回原英雄。若仍要继续，请输入：我确认慈善重随");
    if (confirmation !== "我确认慈善重随") { setMessage("已取消慈善重随。"); return; }
    try { await applyStatusResult(await charityRerollLeagueChampion(confirmation), "慈善重随已完成。"); }
    catch (error) { setMessage(error?.response?.data?.detail || error?.message || "慈善重随失败"); }
  };
  const startDodgeLoop = async () => {
    if (!requireAccountActions()) return;
    setDodgeConfirmOpen(true);
  };
  const confirmDodgeLoop = async () => {
    if (!requireAccountActions() || dodgeSubmitting) return;
    setDodgeSubmitting(true);
    try {
      await applyStatusResult(await startLeagueDodgeLoop("我确认秒退"), "秒退请求已启动。");
      setDodgeConfirmOpen(false);
    } catch (error) {
      setMessage(error?.response?.data?.detail || error?.message || "秒退启动失败");
    } finally {
      setDodgeSubmitting(false);
    }
  };
  const cancelDodgeConfirmation = () => {
    if (dodgeSubmitting) return;
    setDodgeConfirmOpen(false);
    setMessage("已取消秒退。");
  };
  const cancelDodgeLoop = async () => {
    try { await applyStatusResult(await cancelLeagueDodgeLoop(), "连续秒退已取消。"); }
    catch (error) { setMessage(error?.response?.data?.detail || error?.message || "连续秒退取消失败"); }
  };
  const swapBench = async (championId) => {
    if (!requireAccountActions()) return;
    try { await applyStatusResult(await swapLeagueBenchChampion(championId), "已发送备战席换取请求。"); }
    catch (error) { setMessage(error?.response?.data?.detail || "备战席换取失败"); }
  };
  const selectChampionFromMini = async (championId) => {
    if (!requireAccountActions()) return;
    const id = Number(championId);
    if (!Number.isFinite(id) || id <= 0) return;
    try { await applyStatusResult(await selectLeagueChampionFromMini(id), "已发送英雄选择请求。"); }
    catch (error) { setMessage(error?.response?.data?.detail || error?.message || "英雄选择失败"); }
  };
  const selectSkin = async (skinId) => {
    if (!requireAccountActions()) return;
    const id = Number(skinId);
    if (!Number.isFinite(id) || id <= 0) return;
    try { await applyStatusResult(await selectLeagueChampionSkin(id), "已发送皮肤选择请求。"); }
    catch (error) { setMessage(error?.response?.data?.detail || "皮肤选择失败"); }
  };
  const compactDescription = {
    None: status?.requires_elevation ? "检测到客户端，但读取状态需要管理员权限。" : status?.last_error ? `连接失败：${status.last_error}` : "启动并登录客户端后自动连接。",
    Lobby: "房间已连接，等待队友或开始匹配。",
    Matchmaking: "正在搜索对局。",
    ReadyCheck: "对局已找到，请在客户端接受窗口结束前处理。",
    ChampSelect: "英雄选择进行中。",
    InProgress: "游戏进行中。",
    Reconnect: "等待重新连接。",
    PreEndOfGame: "对局正在结算。",
    EndOfGame: "对局已结束。",
    WaitingForStats: "等待战绩返回。",
  }[phase] || "等待客户端状态。";
  return <LeagueMiniView
    status={status}
    phase={phase}
    now={now}
    pinned={pinned}
    message={message}
    canWrite={canWrite}
    dodgeConfirmOpen={dodgeConfirmOpen}
    dodgeSubmitting={dodgeSubmitting}
    visibleSummonerName={visibleSummonerName}
    readyCheck={readyCheck}
    matchmakingSearch={matchmakingSearch}
    actionPlan={actionPlan}
    currentChampionId={currentChampionId}
    team={team}
    bench={bench}
    skinSelector={skinSelector}
    trades={trades}
    respawn={respawn}
    actionCountdown={actionCountdown}
    actionSeconds={actionSeconds}
    phaseSeconds={phaseSeconds}
    compactDescription={compactDescription}
    setWindowPinned={setWindowPinned}
    load={load}
    minimizeWindow={minimizeWindow}
    closeWindow={closeWindow}
    update={update}
    accept={accept}
    declineReady={declineReady}
    cancelAutoAccept={cancelAutoAccept}
    stopMatchmaking={stopMatchmaking}
    applyTradeAction={applyTradeAction}
    reroll={reroll}
    charityReroll={charityReroll}
    swapBench={swapBench}
    selectChampionFromMini={selectChampionFromMini}
    selectSkin={selectSkin}
    startDodgeLoop={startDodgeLoop}
    confirmDodgeLoop={confirmDodgeLoop}
    cancelDodgeConfirmation={cancelDodgeConfirmation}
    cancelDodgeLoop={cancelDodgeLoop}
    setTemporarilyDisabled={async (value) => {
      try { await applyStatusResult(await setLeagueAutoSelectTemporarilyDisabled(value)); }
      catch (error) { setMessage(error?.response?.data?.detail || "切换失败"); }
    }}
  />;
}
