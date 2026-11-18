import { useEffect, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { Download, Gauge, Network, Upload } from "lucide-react";
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";
import {
  downloadJobPercent,
  formatTransferBytes,
  useDemoDownloadStore,
} from "../stores/demoDownloadStore.js";

async function updateTaskbarProgress(job) {
  if (!window.__TAURI_INTERNALS__) return;
  try {
    await getCurrentWindow().setProgressBar(job ? {
      status: job.stage === "resolving" ? ProgressBarStatus.Indeterminate : ProgressBarStatus.Normal,
      progress: downloadJobPercent(job),
    } : { status: ProgressBarStatus.None, progress: 0 });
  } catch {
    // Browser development mode and unsupported platforms simply skip taskbar progress.
  }
}

export default function DemoDownloadActivityCenter() {
  const navigate = useNavigate();
  const jobs = useDemoDownloadStore((state) => state.jobs);
  const refreshJobs = useDemoDownloadStore((state) => state.refreshJobs);
  const active = useMemo(() => jobs.filter((job) => job.status === "running"), [jobs]);
  const current = active[0] || null;
  const hasActiveJobs = active.length > 0;

  useEffect(() => {
    void refreshJobs();
    const resume = () => { if (!document.hidden) void refreshJobs(); };
    document.addEventListener("visibilitychange", resume);
    return () => {
      document.removeEventListener("visibilitychange", resume);
      void updateTaskbarProgress(null);
    };
  }, [refreshJobs]);

  useEffect(() => {
    if (!hasActiveJobs) return;
    let stopped = false;
    const poll = async () => {
      await refreshJobs();
      if (!stopped) timer = window.setTimeout(poll, document.hidden ? 1200 : 500);
    };
    let timer = window.setTimeout(poll, document.hidden ? 1200 : 500);
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [refreshJobs, hasActiveJobs]);

  useEffect(() => { void updateTaskbarProgress(current); }, [current?.job_id, current?.progress, current?.stage]);
  if (!current) return null;

  const percent = downloadJobPercent(current);
  return (
    <button
      type="button"
      onClick={() => navigate("/match-history")}
      className="fixed right-4 top-14 z-[95] w-[360px] rounded-xl border border-cs2-accent/30 bg-cs2-bg-elevated/95 p-3 text-left shadow-2xl backdrop-blur-xl transition-[border-color,transform] duration-150 ease-out hover:border-cs2-accent/60 active:scale-[0.985]"
      aria-label="查看官匹 Demo 下载进度"
    >
      <div className="flex items-start gap-2.5">
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-cs2-accent-soft text-cs2-accent"><Download className="h-4 w-4" /></span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center justify-between gap-3"><span className="truncate text-[11px] font-bold text-cs2-text-primary">{current.filename || "正在解析官匹 Demo"}{active.length > 1 ? ` · 另有 ${active.length - 1} 项` : ""}</span><span className="font-mono text-[10px] font-bold text-cs2-accent">{percent}%</span></span>
          <span className="mt-0.5 block truncate text-[9px] text-cs2-text-muted">{current.message}</span>
        </span>
      </div>
      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-cs2-border"><div className="h-full rounded-full bg-cs2-accent transition-[width] duration-150 ease-out" style={{ width: `${percent}%` }} /></div>
      <div className="mt-2 grid grid-cols-3 gap-2 font-mono text-[8px] text-cs2-text-muted">
        <span className="flex items-center gap-1"><Gauge className="h-3 w-3" />{formatTransferBytes(current.download_speed_bps)}/s</span>
        <span className="flex items-center gap-1"><Network className="h-3 w-3" />↓ {formatTransferBytes(current.download_bytes)}</span>
        <span className="flex items-center justify-end gap-1"><Upload className="h-3 w-3" />↑ {formatTransferBytes(current.upload_bytes)}</span>
      </div>
    </button>
  );
}
