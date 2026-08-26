import React, { Suspense, lazy, useEffect } from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { useThemeStore } from "./stores/themeStore";

const LeagueMiniPanel = lazy(() => import("./pages/LeagueMiniPanel"));

class MiniErrorBoundary extends React.Component {
  constructor(props) { super(props); this.state = { error: null }; }
  static getDerivedStateFromError(error) { return { error }; }
  render() {
    if (this.state.error) return <div className="h-screen bg-[#111214] p-5 text-sm text-red-300"><div className="font-bold">Mini 面板加载失败</div><div className="mt-2 break-words text-xs text-zinc-400">{String(this.state.error?.message || this.state.error)}</div></div>;
    return this.props.children;
  }
}

class MainErrorBoundary extends React.Component {
  constructor(props) { super(props); this.state = { error: null }; }
  static getDerivedStateFromError(error) { return { error }; }
  componentDidCatch(error, info) {
    console.error("[Main Window] Render failed", error, info);
  }
  render() {
    if (!this.state.error) return this.props.children;
    return <div className="grid min-h-screen place-items-center bg-[#101114] p-8 text-zinc-100">
      <div className="w-full max-w-lg rounded-2xl border border-red-400/25 bg-[#17191d] p-6 shadow-2xl">
        <div className="text-base font-bold text-red-300">界面渲染出现异常</div>
        <div className="mt-2 text-sm leading-6 text-zinc-400">主窗口没有消失，点击重新加载即可恢复；错误信息已保留在本机日志中。</div>
        <div className="mt-3 max-h-32 overflow-auto rounded-lg bg-black/25 p-3 font-mono text-xs text-zinc-500">{String(this.state.error?.message || this.state.error)}</div>
        <button type="button" onClick={() => window.location.reload()} className="mt-4 rounded-lg bg-sky-500 px-4 py-2 text-sm font-semibold text-white">重新加载界面</button>
      </div>
    </div>;
  }
}

function ThemeApplier() {
  const mode = useThemeStore((state) => state.mode);
  const setResolvedTheme = useThemeStore((state) => state.setResolvedTheme);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const applyTheme = () => {
      const resolved = mode === "time"
        ? ((new Date().getHours() >= 7 && new Date().getHours() < 19) ? "light" : "dark")
        : mode === "system"
          ? (media.matches ? "dark" : "light")
          : mode;

      document.documentElement.classList.remove("dark", "light");
      document.documentElement.classList.add(resolved);
      document.documentElement.dataset.themeMode = mode;
      document.documentElement.style.colorScheme = resolved;
      setResolvedTheme(resolved);
    };

    applyTheme();
    media.addEventListener?.("change", applyTheme);
    const timer = mode === "time" ? window.setInterval(applyTheme, 60_000) : null;
    return () => {
      media.removeEventListener?.("change", applyTheme);
      if (timer !== null) window.clearInterval(timer);
    };
  }, [mode, setResolvedTheme]);
  return null;
}

const auxiliaryWindow = new URLSearchParams(window.location.search).get("window");
const isLeagueMini = window.__INSIGHT_WINDOW_LABEL__ === "league-mini"
  || auxiliaryWindow === "league-mini";
ReactDOM.createRoot(document.getElementById("root")).render(
  <BrowserRouter>
    <ThemeApplier />
    {isLeagueMini
      ? <MiniErrorBoundary><Suspense fallback={<div className="h-screen bg-[#111214]" />}><LeagueMiniPanel /></Suspense></MiniErrorBoundary>
      : <MainErrorBoundary><App /></MainErrorBoundary>}
  </BrowserRouter>,
);
