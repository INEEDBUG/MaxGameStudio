import React from "react";
import ReactDOM from "react-dom/client";
import { flushSync } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import "./index.css";
import { setDesktopSessionToken } from "./api/api.js";
import LeagueMiniPanel from "./pages/LeagueMiniPanel";

class MiniErrorBoundary extends React.Component {
  constructor(props) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return <div className="h-screen bg-[#111214] p-5 text-sm text-red-300"><div className="font-bold">Mini 面板加载失败</div><div className="mt-2 break-words text-xs text-zinc-400">{String(this.state.error?.message || this.state.error)}</div></div>;
    }
    return this.props.children;
  }
}

function markReadyAfterPaint() {
  const notify = () => invoke("mark_league_window_ready", { kind: "mini" }).catch(() => {});
  // flushSync commits the DOM synchronously, but WebView2 can expose its
  // native surface before that commit has been painted.
  if (typeof window.requestAnimationFrame !== "function") {
    notify();
    return;
  }
  window.requestAnimationFrame(() => window.requestAnimationFrame(notify));
}

async function bootstrap() {
  try {
    const token = await invoke("backend_session_token");
    setDesktopSessionToken(token);
    flushSync(() => root.render(
      <MiniErrorBoundary><LeagueMiniPanel /></MiniErrorBoundary>,
    ));
  } catch (error) {
    flushSync(() => root.render(
      <div className="h-screen bg-[#111214] p-5 text-sm text-red-300"><div className="font-bold">Mini 启动失败</div><div className="mt-2 break-words text-xs text-zinc-400">{String(error?.message || error)}</div><div className="mt-3 text-xs text-zinc-500">请确认主程序后端已启动；关闭并在下一游戏阶段重新打开 Mini 即可重试。</div></div>,
    ));
  } finally {
    // The native window stays hidden until this acknowledgement. It is sent
    // for both the panel and the dark error surface, so failures are visible
    // without exposing WebView2's unpainted white surface.
    markReadyAfterPaint();
  }
}

const rootElement = document.getElementById("root");
const root = ReactDOM.createRoot(rootElement);
flushSync(() => root.render(
  <div className="grid h-screen place-items-center bg-[#111214] p-5 text-center text-sm text-zinc-300">
    <div><div className="mx-auto mb-3 h-6 w-6 animate-spin rounded-full border-2 border-emerald-400/20 border-t-emerald-400" /><div className="font-semibold">正在连接本机服务</div><div className="mt-1 text-xs text-zinc-500">Mini 面板会在连接完成后自动显示状态</div></div>
  </div>,
));
rootElement.dataset.reactMounted = "true";

bootstrap();
