import React from "react";
import ReactDOM from "react-dom/client";
import { flushSync } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import "./index.css";
import { setDesktopSessionToken } from "./api/api.js";
import LeagueCooldownTimerPanel from "./pages/LeagueCooldownTimerPanel";

class CooldownErrorBoundary extends React.Component {
  constructor(props) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return <div className="min-h-screen bg-[#151518] p-3 text-xs text-rose-300"><div className="font-bold">技能计时器加载失败</div><div className="mt-2 break-words text-zinc-400">{String(this.state.error?.message || this.state.error)}</div></div>;
    }
    return this.props.children;
  }
}

const rootElement = document.getElementById("root");
const root = ReactDOM.createRoot(rootElement);
const render = (element) => flushSync(() => root.render(element));
const markReady = () => {
  const notify = () => invoke("mark_league_window_ready", { kind: "cooldown" }).catch(() => {});
  if (typeof window.requestAnimationFrame !== "function") return notify();
  window.requestAnimationFrame(() => window.requestAnimationFrame(notify));
};

render(
  <div className="grid min-h-screen place-items-center bg-[#151518] p-3 text-center text-xs text-zinc-400">
    正在启动技能计时器…
  </div>,
);
rootElement.dataset.reactMounted = "true";

async function bootstrap() {
  try {
    setDesktopSessionToken(await invoke("backend_session_token"));
    render(<CooldownErrorBoundary><LeagueCooldownTimerPanel /></CooldownErrorBoundary>);
  } catch (error) {
    render(<div className="min-h-screen bg-[#151518] p-3 text-xs text-rose-300"><div className="font-bold">技能计时器启动失败</div><div className="mt-2 break-words text-zinc-400">{String(error?.message || error)}</div><div className="mt-2 text-zinc-500">请确认主程序后端已启动；关闭并在下一游戏阶段重新打开技能计时器即可重试。</div></div>);
  } finally {
    // Signal after the normal or error React surface has committed.
    markReady();
  }
}

bootstrap();
