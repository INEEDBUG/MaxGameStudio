import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowRightLeft, RefreshCw } from "lucide-react";
import { desktopBridge } from "../desktop/desktopBridge.js";
import { useLocaleStore } from "../i18n/localeStore.js";
import GameIcon from "./GameIcon.jsx";

const zh = {
  title: "Steam 账户", manage: "管理", refresh: "刷新", detecting: "正在检测 Steam…",
  unsupported: "请在 MaxGameStudio 桌面版使用 Steam 账号切换。",
  missing: "未检测到 Steam", missingHelp: "安装或打开 Steam 后，点击刷新重新检测。",
  empty: "还没有本机账号", emptyHelp: "在 Steam 登录并选择记住登录信息，然后刷新此列表。",
  current: "当前登录", inUse: "正在使用", saved: "已保存登录", signIn: "请在 Steam 登录并保存登录信息",
  switch: "切换账号", switching: "正在切换…", signed_in: "已登录选中的 Steam 账号。",
  already_current: "Steam 已登录这个账号。", waiting_login: "Steam 已启动，请在 Steam 窗口查看登录状态。",
  installPath: "Steam 安装位置", gameRunning: "请在退出 Steam 游戏后切换账号。",
  steam_invalid_accounts: "Steam 的本机账号配置无法读取。请打开 Steam 确认账号后再刷新。",
  steam_unknown_account: "这个账号已不在 Steam 本机列表中，请刷新。",
  steam_not_remembered: "请先在 Steam 为这个账号启用记住登录信息。",
  steam_game_running: "检测到 Steam 正在运行游戏。退出游戏后再试。",
  steam_shutdown_failed: "无法关闭 Steam。请在 Steam 中退出客户端后再试。",
  steam_shutdown_timeout: "Steam 尚未退出。等游戏退出或云同步完成后再试。",
  steam_state_changed: "Steam 状态发生变化，请退出 Steam 后重试。",
  steam_rollback_failed: "切换未完成，无法恢复全部登录选择。请在 Steam 中确认账号。",
  steam_launch_failed: "未能启动 Steam。请手动打开 Steam 后重试。",
  steam_switch_in_progress: "正在切换 Steam 账号，请稍候。",
  steam_not_installed: "未检测到 Steam。",
  readFailed: "无法读取 Steam 账户。请刷新后重试。",
  switchFailed: "Steam 账号切换未完成，请检查 Steam 后重试。",
};

const en = {
  title: "Steam accounts", manage: "Manage", refresh: "Refresh", detecting: "Checking for Steam…",
  unsupported: "Open MaxGameStudio desktop to switch Steam accounts.",
  missing: "Steam was not detected", missingHelp: "Install or open Steam, then refresh to check again.",
  empty: "No accounts on this PC", emptyHelp: "Sign in to Steam, choose to remember the sign-in, and refresh this list.",
  current: "Signed in", inUse: "In use", saved: "Saved sign-in", signIn: "Sign in and save your login in Steam",
  switch: "Switch account", switching: "Switching…", signed_in: "Signed in to the selected Steam account.",
  already_current: "Steam is already signed in to this account.", waiting_login: "Steam has started. Check the sign-in status in Steam.",
  installPath: "Steam installation", gameRunning: "Close your Steam game before switching accounts.",
  steam_invalid_accounts: "The local Steam account configuration could not be read. Open Steam, check your account, and refresh.",
  steam_unknown_account: "This account is no longer in Steam’s local list. Refresh to update the list.",
  steam_not_remembered: "Enable the saved sign-in option for this account in Steam first.",
  steam_game_running: "A Steam game is running. Close it and try again.",
  steam_shutdown_failed: "Steam could not be closed. Exit it from Steam and try again.",
  steam_shutdown_timeout: "Steam is still open. Wait for your game to exit or cloud sync to finish, then try again.",
  steam_state_changed: "Steam’s state changed. Exit Steam and try again.",
  steam_rollback_failed: "The switch did not finish, and some login preferences could not be restored. Check your account in Steam.",
  steam_launch_failed: "Steam could not be started. Open Steam manually and try again.",
  steam_switch_in_progress: "Steam is already switching accounts. Wait until it finishes.",
  steam_not_installed: "Steam was not detected.",
  readFailed: "Steam accounts could not be read. Refresh and try again.",
  switchFailed: "The Steam account switch did not finish. Check Steam and try again.",
};

const frame = "rounded-lg border border-cs2-border bg-cs2-bg-card";
const button = "inline-flex min-h-8 shrink-0 items-center justify-center gap-1.5 rounded-md border border-cs2-border px-3 py-1.5 text-xs font-medium focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cs2-accent disabled:cursor-default disabled:opacity-60";

export default function SteamAccountsPanel({ compact = false }) {
  const locale = useLocaleStore((state) => state.effectiveLocale);
  const strings = locale?.startsWith("zh") ? zh : en;
  const headingId = useId();
  const nextRequest = useRef(0);
  const operation = useRef(false);
  const [snapshot, setSnapshot] = useState(null);
  const [loading, setLoading] = useState(Boolean(desktopBridge?.getSteamAccounts));
  const [target, setTarget] = useState(null);
  const [failure, setFailure] = useState(null);
  const [feedback, setFeedback] = useState(null);

  const refresh = useCallback(async (announce = false) => {
    if (!desktopBridge?.getSteamAccounts) return;
    const request = ++nextRequest.current;
    if (announce) setLoading(true);
    try {
      const data = await desktopBridge.getSteamAccounts();
      if (request !== nextRequest.current) return;
      setSnapshot(data);
      setFailure((previous) => (announce || previous?.fallback === "readFailed" ? null : previous));
    } catch (error) {
      if (request !== nextRequest.current) return;
      setSnapshot(null);
      setFailure({ code: String(error), fallback: "readFailed" });
    } finally {
      if (request === nextRequest.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!desktopBridge?.getSteamAccounts) return undefined;
    void refresh(true);
    const onFocus = () => { if (!operation.current) void refresh(); };
    window.addEventListener("focus", onFocus);
    return () => {
      nextRequest.current += 1;
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  async function switchTo(steamId) {
    if (operation.current) return;
    operation.current = true;
    nextRequest.current += 1;
    setTarget(steamId);
    setFailure(null);
    setFeedback(null);
    try {
      const result = await desktopBridge.switchSteamAccount(steamId);
      if (result.status === "waiting_login") {
        setSnapshot((previous) => previous && { ...previous, current_steam_id: null });
      }
      setFeedback(result);
    } catch (error) {
      setFailure({ code: String(error), fallback: "switchFailed" });
    } finally {
      await refresh();
      operation.current = false;
      setTarget(null);
    }
  }

  const Heading = compact ? "h2" : "h1";
  const accounts = snapshot?.accounts ?? [];
  const feedbackStatus = feedback?.status === "waiting_login" && snapshot?.current_steam_id === feedback.steam_id
    ? "signed_in"
    : feedback?.status;

  return (
    <section aria-labelledby={headingId}>
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2.5">
          <GameIcon game="steam" className="h-6 w-6 shrink-0" />
          <Heading id={headingId} className={compact ? "text-[15px] font-semibold" : "text-xl font-semibold"}>{strings.title}</Heading>
        </div>
        {compact && <Link to="/steam" className={`${button} bg-cs2-bg-card text-cs2-text-secondary hover:bg-cs2-bg-hover`}>{strings.manage}</Link>}
        {!compact && desktopBridge?.getSteamAccounts && (
          <button type="button" className={`${button} bg-cs2-bg-card text-cs2-text-secondary hover:bg-cs2-bg-hover`} disabled={loading || Boolean(target)} onClick={() => { setFeedback(null); void refresh(true); }}>
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />{strings.refresh}
          </button>
        )}
      </div>

      {!desktopBridge?.getSteamAccounts && <p className={`${frame} p-3.5 text-sm text-cs2-text-secondary`}>{strings.unsupported}</p>}
      {loading && !snapshot && <p className={`${frame} p-3.5 text-sm text-cs2-text-secondary`} role="status">{strings.detecting}</p>}
      {snapshot && (!snapshot.installed || accounts.length === 0) && (
        <div className={`${frame} space-y-1.5 p-3.5`}>
          <p className="text-sm font-medium text-cs2-text-primary">{snapshot.installed ? strings.empty : strings.missing}</p>
          <p className="text-xs leading-5 text-cs2-text-secondary">{snapshot.installed ? strings.emptyHelp : strings.missingHelp}</p>
        </div>
      )}
      {Boolean(accounts.length) && (
        <ul className={`${frame} divide-y divide-cs2-border px-3.5`}>
          {accounts.map((account) => {
            const current = account.steam_id === snapshot.current_steam_id;
            const name = account.persona_name || account.account_name;
            const status = current ? strings.current : account.remembered ? strings.saved : strings.signIn;
            const nameId = `${headingId}-${account.steam_id}`;
            return (
              <li key={account.steam_id} className="flex items-center justify-between gap-3 py-3">
                <div className="flex min-w-0 items-center gap-2.5">
                  <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md bg-cs2-bg-hover text-xs font-medium text-cs2-text-secondary" aria-hidden="true">{Array.from(name)[0]?.toLocaleUpperCase()}</span>
                  <div className="min-w-0"><p id={nameId} className="truncate text-sm font-medium text-cs2-text-primary">{name}</p><p className="text-xs leading-5 text-cs2-text-secondary">{status}</p></div>
                </div>
                {(current || account.remembered) && (
                  <button type="button" aria-describedby={nameId} className={`${button} ${current ? "text-cs2-text-secondary" : "border-transparent bg-cs2-accent text-cs2-text-on-accent enabled:hover:brightness-110"}`} disabled={current || Boolean(target) || snapshot.game_running} onClick={() => void switchTo(account.steam_id)}>
                    {!current && <ArrowRightLeft className="h-3.5 w-3.5" aria-hidden="true" />}
                    {current ? strings.inUse : target === account.steam_id ? strings.switching : strings.switch}
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {snapshot?.game_running && <p className="mt-2 text-xs text-cs2-text-secondary">{strings.gameRunning}</p>}
      {failure && <p role="alert" className="mt-2 text-xs text-cs2-text-error">{Object.hasOwn(strings, failure.code) ? strings[failure.code] : strings[failure.fallback]}</p>}
      {feedbackStatus && <p role="status" className="mt-2 text-xs text-cs2-text-secondary">{strings[feedbackStatus]}</p>}
      {!compact && snapshot?.steam_path && <p className="mt-3 break-all text-xs text-cs2-text-secondary">{strings.installPath} · {snapshot.steam_path}</p>}
    </section>
  );
}
