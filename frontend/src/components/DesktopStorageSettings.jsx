import { useCallback, useEffect, useRef, useState } from "react";
import { HardDrive, Loader2, RefreshCw, ShieldCheck } from "lucide-react";
import { desktopBridge } from "../desktop/desktopBridge.js";

function formatBytes(value) {
  const bytes = Number(value);
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let amount = bytes;
  let unit = -1;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }
  return `${amount.toFixed(amount >= 10 ? 0 : 1)} ${units[unit]}`;
}

function PathLine({ label, path, bytes }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 text-[11px]">
      <span className="text-cs2-text-muted">{label}</span>
      <span className="min-w-0 break-all text-right font-mono text-cs2-text-secondary">
        {path || "—"}{typeof bytes === "number" && Number.isFinite(bytes) && bytes >= 0 ? ` · ${formatBytes(bytes)}` : ""}
      </span>
    </div>
  );
}

export default function DesktopStorageSettings({ search = false }) {
  const [storage, setStorage] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const busyRef = useRef(false);
  const mountedRef = useRef(true);
  const requestRef = useRef(0);

  const request = useCallback(async (operation) => {
    if (!desktopBridge || busyRef.current) return;
    const requestId = requestRef.current + 1;
    requestRef.current = requestId;
    busyRef.current = true;
    setBusy(true);
    setError("");
    try {
      const value = await operation();
      if (mountedRef.current && requestRef.current === requestId) setStorage(value);
    } catch (cause) {
      if (mountedRef.current && requestRef.current === requestId) setError(String(cause?.message || cause));
    } finally {
      if (mountedRef.current && requestRef.current === requestId) {
        busyRef.current = false;
        setBusy(false);
      }
    }
  }, []);

  const refresh = useCallback(() => request(() => desktopBridge.getDesktopStorage()), [request]);

  useEffect(() => {
    mountedRef.current = true;
    if (desktopBridge?.getDesktopStorage) {
      void request(() => desktopBridge.getDesktopStorage());
    }
    return () => {
      mountedRef.current = false;
      requestRef.current += 1;
      busyRef.current = false;
    };
  }, [request]);

  const choose = useCallback(() => request(() => desktopBridge.chooseDesktopStorage()), [request]);

  const cancel = useCallback(() => request(() => desktopBridge.cancelDesktopStorageChange()), [request]);

  if (!desktopBridge || search) return null;

  const pending = Boolean(storage?.pending);
  return (
    <div className="border-b border-cs2-border/40 py-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2">
          <HardDrive className="mt-0.5 h-4 w-4 shrink-0 text-cs2-accent" aria-hidden="true" />
          <div>
            <h3 className="text-xs font-semibold text-cs2-text-secondary">数据存储位置 / Data storage location</h3>
            <p className="mt-0.5 max-w-2xl text-[11px] leading-relaxed text-cs2-text-muted">
              这里仅显示应用数据、日志、缓存、WebView、英雄联盟运行时和临时文件的实际位置。应用不会在启动时自动搬运旧数据；使用“更改存储位置”后，新的目录在重启后生效。
            </p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button type="button" onClick={() => void refresh()} disabled={busy} aria-label="刷新存储状态" className="rounded-md border border-cs2-border bg-cs2-bg-input p-2 text-cs2-text-muted hover:text-cs2-accent disabled:opacity-50">
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
          <button type="button" onClick={() => void choose()} disabled={busy} className="inline-flex items-center gap-1.5 rounded-md bg-cs2-accent px-3 py-2 text-xs font-bold text-cs2-text-on-accent hover:bg-cs2-accent-light disabled:opacity-50">
            {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" /> : null}
            更改存储位置
          </button>
          {pending ? <button type="button" onClick={() => void cancel()} disabled={busy} className="rounded-md border border-cs2-border px-3 py-2 text-xs text-cs2-text-secondary hover:border-rose-400/50 hover:text-rose-300 disabled:opacity-50">取消待处理更改</button> : null}
        </div>
      </div>
      <div className="mt-3 space-y-1.5 rounded-md border border-cs2-border/60 bg-cs2-bg-input/40 p-2.5">
        <PathLine label="普通数据根目录（不含管理员目录）" path={storage?.root} bytes={storage?.bytes} />
        {storage?.paths ? <>
          <PathLine label={storage.mode === "legacy_in_place" ? "数据（旧位置）" : "数据"} path={storage.paths.data} />
          <PathLine label={storage.mode === "legacy_in_place" ? "日志（旧位置）" : "日志"} path={storage.paths.logs} />
          <PathLine label={storage.mode === "legacy_in_place" ? "缓存（旧位置）" : "缓存"} path={storage.paths.cache} />
          <PathLine label={storage.mode === "legacy_in_place" ? "WebView（旧位置）" : "WebView"} path={storage.paths.webview} />
          <PathLine label={storage.mode === "legacy_in_place" ? "英雄联盟运行时（旧位置）" : "英雄联盟运行时"} path={storage.paths.league_runtime} />
          <PathLine label={storage.mode === "legacy_in_place" ? "临时文件（旧位置）" : "临时文件"} path={storage.paths.temp} />
        </> : null}
        {storage?.protected_root ? <PathLine label="管理员受保护目录（按需创建）" path={storage.protected_root} /> : null}
        {pending ? <PathLine label="待切换目录" path={storage.pending} bytes={storage.pending_bytes} /> : null}
        {storage?.previous ? <PathLine label="保留的原目录" path={storage.previous} bytes={storage.previous_bytes} /> : null}
      </div>
      {storage?.last_switch_error ? <div role="alert" className="mt-2 break-words rounded-md border border-cs2-border bg-cs2-amber-surface px-3 py-2 text-[11px] leading-relaxed text-cs2-amber-on-surface">{storage.last_switch_error}。可以再次点击“更改存储位置”重试。</div> : null}
      {pending ? <p className="mt-2 text-[11px] text-cs2-amber-on-surface">已安排重启后切换目录。不会自动复制旧数据：原目录保持不变，选择空目录将使用全新设置；选择已有数据目录则继续使用其中的数据。</p> : null}
      {storage?.system_drive ? <p className="mt-2 text-[11px] text-cs2-amber-on-surface">当前目录位于系统盘；建议选择固定非系统盘以避免 C 盘空间和权限问题。</p> : null}
      {error ? <p role="alert" className="mt-2 text-[11px] text-cs2-text-error">{error}</p> : null}
      {!storage && !error ? <p className="mt-2 text-[11px] text-cs2-text-muted">正在读取存储状态…</p> : null}
      <p className="mt-2 flex items-center gap-1 text-[10px] text-cs2-text-muted"><ShieldCheck className="h-3 w-3" aria-hidden="true" />原目录不会被安装器删除。</p>
    </div>
  );
}
