import React from "react";
import ReactDOM from "react-dom/client";
import { flushSync } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide-react";
import "./index.css";
import { setDesktopSessionToken } from "./api/api.js";
import LeagueOngoingGame from "./components/league/LeagueOngoingGame";

function OngoingWindowShell() {
  return <main className="min-h-screen bg-[#111214] p-5 text-white">
    <header data-tauri-drag-region className="mb-3 flex items-center justify-between border-b border-white/10 pb-2 text-xs text-zinc-400">
      <span data-tauri-drag-region>MaxGameStudio · League 实时对局</span>
      <span className="flex items-center gap-1">
        <button type="button" aria-label="关闭实时对局" onClick={() => getCurrentWindow().close()} className="rounded p-1.5 hover:bg-rose-500 hover:text-white"><X className="h-3.5 w-3.5" /></button>
      </span>
    </header>
    <LeagueOngoingGame onOpenPlayer={() => {}} onError={() => {}} />
  </main>;
}

class OngoingErrorBoundary extends React.Component {
  constructor(props) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return <div className="min-h-screen bg-[#111214] p-5 text-sm text-red-300"><div className="font-bold">实时对局窗口加载失败</div><div className="mt-2 break-words text-xs text-zinc-400">{String(this.state.error?.message || this.state.error)}</div></div>;
    }
    return this.props.children;
  }
}

const rootElement = document.getElementById("root");
const root = ReactDOM.createRoot(rootElement);
const render = (element) => flushSync(() => root.render(element));
const markReady = () => {
  const notify = () => invoke("mark_league_window_ready", { kind: "ongoing" }).catch(() => {});
  if (typeof window.requestAnimationFrame !== "function") return notify();
  window.requestAnimationFrame(() => window.requestAnimationFrame(notify));
};

render(
  <div className="grid min-h-screen place-items-center bg-[#111214] p-5 text-center text-sm text-zinc-300">
    <div><div className="mx-auto mb-3 h-6 w-6 animate-spin rounded-full border-2 border-emerald-400/20 border-t-emerald-400" /><div className="font-semibold">正在启动实时对局窗口</div><div className="mt-1 text-xs text-zinc-500">正在连接本机服务…</div></div>
  </div>,
);
rootElement.dataset.reactMounted = "true";

async function bootstrap() {
  try {
    setDesktopSessionToken(await invoke("backend_session_token"));
    render(<OngoingErrorBoundary><OngoingWindowShell /></OngoingErrorBoundary>);
  } catch (error) {
    render(<div className="min-h-screen bg-[#111214] p-5 text-sm text-red-300"><div className="font-bold">实时对局窗口启动失败</div><div className="mt-2 break-words text-xs text-zinc-400">{String(error?.message || error)}</div><div className="mt-3 text-xs text-zinc-500">请确认主程序后端已启动；关闭并在下一游戏阶段重新打开窗口即可重试。</div></div>);
  } finally {
    // The native shell only becomes visible after this React/error surface is
    // committed, preventing an unpainted WebView from flashing white.
    markReady();
  }
}

bootstrap();
